# RFC: Complete Transition Journal and Recoverable External Access

Status: proposed

Scope: typed state execution, certified specs, run journal, external-access audit, recoverability,
replay, facts, framework enforcement, execution catalogs, and app status

This RFC proposes a breaking replacement of the current event, projection, attempt, side-effect,
and generic saga machinery. It is not the current authoritative contract. If accepted, the
[design contract](docs/design.md), [architecture guide](docs/architecture.md), affected companion
documents, code, schemas, and tests must move to this design together.

## Executive Decision

MFM will have one append-only transition journal as the sole semantic authority for a run.

Every committed state transition will be independently traceable from:

- the exact certified run and node;
- the exact journal head the state observed;
- every typed input binding and its lineage;
- the request or committed external observation it consumed;
- the typed result, outputs, facts, and reviewed error evidence it produced; and
- the resulting run and node state.

A commit batch is only the atomic storage envelope that admits one semantic transition, its
content-addressed objects, and any inseparable companion records. It is not a replacement for
per-state transition history and does not combine unrelated state executions.

External access receives a narrow audit protocol:

1. MFM durably records one `ExternalAccessAuthorized` record.
2. Only a newly appended authorization mints one transient, affine access authority bound to that
   exact run, scope, capability, operation, request, and semantic anchor.
3. One authorization permits at most one boundary entry; if entry occurs, the audited capability
   performs exactly one independently meaningful application-protocol operation and no hidden
   retry, redirect, failover, provider reselection, multi-operation batch, or response-dependent
   subcall.
4. MFM durably records the one `ExternalAccessObserved` outcome whenever the wrapper result
   survives; loss of the executing task or process, or an unresolved append, can leave it absent.
5. A semantic transition may consume only a committed, reviewed observation.

This protocol proves:

```text
MFM-controlled application-protocol operation
    implies
a matching durable authorization already existed
```

It deliberately cannot prove the converse. A process can crash after authorization and before
entering the external boundary. An authorization without an observation remains `CrashAmbiguous`
unless and until its one matching observation commits: zero or one physical invocation may have
occurred, and no successor may synthesize an outcome.

Writes use one stronger and smaller recovery primitive:

```text
ensure(committed effect key, immutable semantic request)
    -> Pending(delivery audit) | Terminal(typed evidence containing delivery audit)
```

Repeated, concurrent, and delayed `ensure` calls must converge on one logical external effect. A
destination without a native form of that property must sit behind a durable keyed executor with
an independently reviewed downstream convergence mechanism, or remain unsupported as a certified
effect state. A keyed ledger alone is not that mechanism.

Generic saga, compensation, manual resolution, phase ledgers, worker attempts, resource lanes,
recovery modes, and the public universal projection are removed. Domain correction is expressed as
a separately certified run consuming immutable source-transition evidence.

The runtime becomes one stateless `drive_once` interpreter over three closed state protocols:
pure, external read, and recoverable effect. Extensibility means adding typed states, values,
capabilities, and executors, not adding runner kinds or lifecycle phases. The runtime obtains live
evidence only for a read or pending effect that lacks sufficient committed evidence; whenever it
does invoke a live capability, authorization before entry and observation after every surviving
wrapper result are mandatory, never optional.

Framework pre/post semantics do not add a runtime envelope. An exact entry-point profile injects
ordinary typed states into the authored program before certification, certification reruns that
pure planner, and runtime executes only the resulting ordinary graph. All other invalid
authority-bearing states are unrepresentable after sealed construction or rejected once at an
unavoidable decode, evidence, or store boundary.

## Problem Situation

### Transition history is fragmented instead of being the primitive

The current run stream is authoritative, but one state evaluation is represented through several
event families and then reconstructed through broad projections and runtime-specific maps. Exact
inputs exist during materialization, while outputs, facts, evidence, attempts, side-effect phases,
public output, and completion are persisted separately.

This makes a basic question unnecessarily expensive:

> Given one state execution, what exact information did it consume, what external evidence did it
> accept, and what did it produce?

The answer should be one first-class transition record, not a join across attempt, cell, fact,
artifact, phase, and completion models.

### Projection became a second model of the runtime

The current projection surface collects admission, attempts, cells, side-effect phases, saga
state, resource claims, retention, public output, and terminal state into a broadly shared
snapshot. Runtime and replay then construct additional lifecycle maps over the same history.

The consequences are:

- a lifecycle change crosses store, runtime, replay, app, and transport-facing status code;
- callers can accidentally treat derived maps as independent semantic authority;
- independently loaded views can represent different stream watermarks;
- protocol-specific mutation phases become domain-free projection fields;
- physical indexes, transient folds, and operational coordination are all called projections even
  though they have different correctness roles; and
- input/output lineage exists, but it is not exposed as one direct transition trace.

Projection remains useful as a transient folding technique. A public universal projection model is
the problem.

### External access failures cross the typed boundary

Types, certification, and store validation can make many internal invalid states unrepresentable.
They cannot make a live provider, filesystem, network, signer, remote executor, or external system
available or well behaved.

An external access can:

- reject before boundary entry;
- return a reviewed typed result;
- return malformed or unrepresentable data;
- time out after the external system acted;
- fail after sending but before returning a response;
- be interrupted by process death; or
- return after another worker has already settled the state.

These are auditable facts about MFM's interaction boundary. They must not disappear merely because
they did not produce a typed state output.

The current worker-attempt lifecycle is too broad for this purpose. It records process driving and
is entangled with node recovery, side-effect ownership, claims, and terminalization. The needed
primitive is narrower: immutable authorization and observation records around every audited live
application-protocol operation.

### Recoverability is distributed across too many phases

The current mutation lifecycle distinguishes intent, claim, prepared invocation, invocation
started, submission known or unknown, receipt, confirmation, ambiguity, failure, attempt recovery,
resource lanes, and saga recovery.

These distinctions create a large phase cross-product without establishing generic exactly-once
external execution. Attempt identity, effect identity, pair identity, claim ownership, and external
operation identity can drift apart. Fixing one edge requires coordinated edits across events,
store typestate, projections, scheduling, replay, adapters, APIs, and tests.

The core should own only:

- one immutable semantic request;
- one stable effect key;
- one pending effect state;
- audited calls to a certified keyed executor; and
- one evidence-verified terminal state transition.

Protocol delivery, nonce or UTXO selection, signing, replacement, provider failover, and
destination-specific evidence progression belong to the effect executor and its certified
contract.

### Generic saga policy remains too large for the provable claim

The current [certified saga contract](docs/saga.md) honestly narrows AC/DC claims. It nevertheless
requires the kernel to understand forward and remediation roles, pair linkage, engagement,
quiescence, obligations, reverse remediation order, run modes, manual authorization, and public
compensation outcomes.

A generic kernel cannot prove that an arbitrary external compensation restored business
equivalence. A signed operator decision proves authorization to make a decision, not external
domain truth.

Correction remains necessary, but it is domain behavior. A refund, revocation, bridge recovery,
replacement transaction, or governance correction has its own preconditions, evidence, and
meaning. It should be a typed operation in a new run, not a generic kernel obligation hidden
inside the source run.

## Goals

- Make complete per-state input/output lineage a first-class persisted primitive.
- Preserve one semantic transition per atomic commit batch.
- Record a distinct authorization before every independently meaningful MFM-controlled external
  read or write application-protocol operation.
- Record every reviewed capability return or safe failure that MFM can durably observe.
- Represent malformed and unrepresentable external results without admitting unsafe raw bytes.
- Keep pure computation and worker scheduling attempts out of semantic history.
- Make one append-only journal the sole run-lifecycle authority.
- Derive status, scheduling, recovery, replay, public output, facts, and analysis from that journal.
- Keep writes as a first-class certified effect through one keyed-convergent `ensure` contract.
- Preserve one canonical run identity across admission retries and acknowledgement ambiguity.
- Preserve stable effect identity across crashes, workers, and retries.
- Prevent a different request from replacing a pending effect.
- Keep state logic pure and all live IO behind audited capabilities.
- Execute every state through one of three closed typed protocols and one generic runtime path.
- Keep all outcome-affecting request authorship, evidence acceptance, reduction, validation,
  failure policy, output construction, and fact emission in the certified state contract.
- Make invalid kernel states unrepresentable behind sealed typed constructors, certification, and
  append authority, with explicit revalidation only where untrusted bytes or concurrency cross a
  trust boundary.
- Express framework semantic pre/post behavior as ordinary typed states injected during
  deterministic plan construction and independently verified by certification.
- Preserve atomic appends, content addressing, canonical JSON, float-free hashed structures, and
  no-secret persisted surfaces.
- Replace generic saga semantics with explicit typed corrective runs.
- Minimize concepts, public types, duplicated folds, future change sites, and lines of code.

## Non-Goals

- Proving that a durable authorization corresponds to exactly one physical network request.
- Auditing the journal store's own database calls, which would create infinite regress.
- Auditing DNS, TLS packets, kernel syscalls, or executor-internal provider calls as MFM journal
  records.
- Collapsing several independently meaningful external operations into one audited capability
  access merely because a transport can batch them.
- Persisting raw provider responses, credentials, signed bearer bytes, or secret-derived hashes.
- Recording every pure computation, worker scheduling, panic, retry loop, or process takeover.
- Exactly-once physical delivery to arbitrary external systems.
- Supporting non-convergent one-shot writes as certified recoverable effects.
- Generic transaction replacement, fee bumping, nonce management, or UTXO selection in the kernel.
- Cross-system atomicity, generic rollback, or generic compensation equivalence.
- Concurrency control over wallets, validators, operators, or applications outside MFM.
- Automatic or guaranteed admission of a corrective follow-up run.
- Extensible runtime lifecycle protocols, custom runner event algebras, or adapter-owned reducers.
- Treating best-effort telemetry callbacks as semantic pre/post execution.
- Replacing kernel/store structural validation with framework-injected states.
- Giving injected framework states a special runtime lifecycle, callback API, or journal record.
- Claiming Rust signatures alone prove compiled callback purity, determinism, audited-only IO, or
  semantic non-secrecy.
- Claiming portable fact-selection completeness before one concrete scope-safe commitment,
  authority, and verifier contract is selected in a later RFC.
- Claiming a later post-state retroactively validates an already committed transition or external
  mutation.
- Runtime-loaded native libraries, WASM states, or remote state-execution ABIs in this cutover.
- Cross-store import of run-source authority.
- Backward compatibility with current event, projection, attempt, saga, or store schemas.

## Material Uncertainties

| Choice or assumption | Why it is uncertain | Consequence if wrong | Resolution or validation |
| --- | --- | --- | --- |
| Direct MFM audit and executor-supplied delivery audit form one reviewable trace. | An affine Rust value cannot control a remote executor's internal HTTP/RPC exchanges. | An unmatched executor call may leave MFM without the latest target-attempt frontier, and consumers might conflate direct authorization with executor attestation. | Require a durable executor authorization before every target attempt, immutable predecessor-linked frontiers retained by MFM with each result, and a proof/attestation basis labeled distinctly from MFM authorization/observation. |
| A durable authorization is acceptable as the honest pre-call audit fact. | No local database protocol can atomically prove that a remote boundary was physically entered. | Consumers could incorrectly read authorization as proof that a request was sent. | Name and document the record as authorization, expose unmatched records as `CrashAmbiguous`, and never claim exact physical delivery. |
| Reviewed typed results and safe failure summaries provide enough audit detail. | Raw provider bodies or error chains may contain useful diagnostics as well as secrets, and semantic secrecy is not decidable from a Rust type alone. | Redaction may omit forensic detail; retaining raw material may violate the no-secret invariant. | Define bounded closed public result/failure schemas per capability, qualify classifiers/canonicalizers, and adversarially test credential, bearer, and low-entropy-secret leakage. |
| Complete transition/audit correlation and retention are acceptable in the run journal. | Exact input lineage, non-public outputs/facts/evidence, redacted access order, frequency, source choice, and wall-clock metadata may reveal tenant behavior or provider incidents. | Indefinite immutable retention or broad trace dereferencing could conflict with privacy, erasure, tenant isolation, or least-access requirements. | Keep status/public-output access separate from privileged tenant/run-authorized trace and object access, omit source and precise time by default, and define the production retention policy before rollout. If deletion is mandatory, resolve the journal-integrity and archival contract in a follow-up RFC rather than silently weakening this trace. |
| Same-tenant cross-run sources and fact selection cover the baseline product. | A future aggregate or delegated analysis may need to consume another tenant's approved data. | Allowing that implicitly would turn a run reference or fact query into cross-tenant bearer authority; forbidding it may constrain future products. | Reject cross-tenant sources and facts in this cutover. If required later, design a separate delegated-access contract with explicit source authority before changing the frozen schemas. |
| Semantic closure may be followed by audit-only observations. | Current terminal models often prohibit every later run append. | A stale call returning after closure could not be recorded, or one crashed call could block closure forever. | Make closure absorbing for semantic transitions and new authorizations while allowing one observation for a pre-closure unmatched authorization. |
| Two durable audit appends per live access are affordable. | High-frequency reads can create substantial latency and journal volume. | The audit primitive may dominate execution cost. | Implement correctness first, measure representative workloads, and optimize only without hiding individual authorization identities or weakening affine access authority. |
| Strict audit availability may gate every live access. | Durable authorization must succeed before the capability becomes callable. | A journal outage becomes a provider-access outage rather than degraded audit coverage. | Keep the protocol fail closed and measure the availability budget. A deployment requiring a best-effort bypass does not implement this contract and cannot register the audited live capability. |
| Database HA can provide non-rollback exclusive writer fencing for one preserved store identity. | A restored clone may otherwise accept writes while an old or sibling primary is still writable. | Identical run/store coordinates could name divergent journals, invalidating facts, cross-run refs, and effect recovery. | Qualify the deployment fence, require proof of the unique latest prefix before promotion, test stale-primary/restore split brain, and fail closed if a published suffix may be lost. |
| Intended mutation executors can provide permanent keyed convergence. | Some systems have finite idempotency windows or no client-chosen identity. | A delayed call could create another mutation or cost-bearing operation. | Inventory executors and certify permanent destination-native convergence or a keyed executor with its own reviewed downstream convergence proof. Unsupported effect states remain unregistered. |
| Executor-owned delivery state is acceptable as an irreducible external authority. | The goal is to remove secondary run-state stores, but an executor ledger still does not create destination convergence by itself. | Mistaking the ledger for a generic idempotency proof could duplicate a downstream mutation after `target applied -> executor crash`. | Treat the ledger as external-delivery authority only and require an independently reviewed downstream convergence mechanism, permanent anti-rollback bindings, and disaster-recovery tests. |
| Cross-effect resource coordination can leave the kernel. | Different effect keys may still compete for one nonce, UTXO, sequence, inventory item, or business resource. | Removing resource lanes without a replacement owner could admit conflicting external operations. | Require the executor/destination to provide exclusive ownership, one shared durable coordinator, or atomic domain preconditions. Keep an effect state unregistered until that ownership is concrete. |
| Explicit per-operation read states are practical at production fan-out. | Current EVM collection can issue many independent reads, while Bitcoin collection contains several response-dependent RPCs. | The chosen audit guarantee may materially increase graph size, journal volume, and latency. | Prototype the exact EVM and Bitcoin target graphs, including cancellation and partial failure, before schema freeze. Measure representative fan-out and optimize only without combining audit identities or moving adaptive request authorship outside typed states. |
| Bitcoin `scantxoutset "start"` qualifies as a repeat-work-safe read. | It initiates costly shared server work even though it does not create a durable domain mutation. | Crash recovery may repeat work or contend with another scan; if that consequence is not bounded and accepted, classifying it as an ordinary read is dishonest. | Prototype lost responses, cancellation, concurrent scans, delayed reissue, bounded descriptors/results, and provider cost. Keep production Bitcoin collection unregistered unless it passes; otherwise design a keyed work executor plus audited status reads without changing the generic runtime. |
| Provider-native multi-operation batching can remain unsupported in the baseline. | A later optimization may want to carry several ready requests in one HTTP or JSON-RPC exchange. | Treating the transport envelope as one audit unit would hide independently meaningful operations; adding batching prematurely would complicate affine authority and partial-observation atomicity. | Baseline capabilities perform one application-protocol operation. A future batching RFC must preserve one authorization and one outcome identity per member and prove consumption, cancellation, and partial-return behavior. A collection-valued method is one operation only when its members have one indivisible semantic identity and shared snapshot/result contract. |
| Read and effect request authorship can be total over certified typed inputs. | Existing planners may accept weak types and report domain errors before producing a request. | A generic pre-call runner-failure path would reintroduce another lifecycle. | Move fallible validation into an upstream pure state that produces a stronger type and property-test request totality. If a legitimate case remains, specify one closed local typed-failure transition rather than generic runner errors. |
| Every kernel invariant can be excluded at the typed execution boundary or enforced at an unavoidable trust boundary. | Persisted bytes, external evidence, catalog erasure, and concurrent appends cannot be made safe by Rust types before they are decoded or admitted. | Calling all invariants “type guaranteed” could hide required replay, store, or evidence verification; adding named validation phases would recreate a lifecycle. | Inventory every invariant. Use sealed constructors and certified types inside the process, and keep decoding, cryptographic verification, exact-head compare-and-swap, and atomic append as boundary predicates rather than framework phases. |
| An exact profile planner can inject typed semantic pre/post states without runtime hooks. | Generic pre-states must preserve heterogeneous input trees, while post-states must gate every consumer/export without changing the protected state's reusable domain contract. | A special `StateFrame` field or runtime branch would return; an incomplete rewrite could bypass policy. | Prototype pure, read, effect, fan-in, and child-operation cases. Prefer typed input pass-through and effective-output rewiring; reject a profile/state combination that cannot be expressed through ordinary state contracts. |
| Every published entry point can bind one exact planning profile. | Library consumers may assemble MFM through other entry points with different policy. | “Minimum profile” or implicit superset semantics would require another policy-ordering algebra and could overstate which guarantees apply. | Bind one content-addressed `PlanningProfile` and planner contract per entry point. A policy change creates a new identity, and guarantees are stated for that exact profile. |
| Certification can retain and re-expand one canonical authored program. | Current certification may retain only the lowered graph. | The certifier could not independently prove that mandatory planning-time injection was applied exactly once. | Bind a content-addressed authored-program artifact, exact planning profile/planner identity, and expanded-spec hash into the certificate; verification reruns the pure planner before minting `CertifiedTypedSpec`. |
| Required post-states need only explicit typed inputs and the protected typed output. | A proposed policy may ask to inspect arbitrary journal or audit internals. | A privileged post context would recreate a second runtime API and couple policy to storage representation. | Inventory concrete postconditions. Keep observation/effect-evidence verification in the producing state and reject generic journal-inspecting post hooks. |
| The generic evidence-reference bag can be removed completely. | Existing states may retain auxiliary evidence outside outputs or facts. | Migration may lose trace material or reintroduce an untyped escape hatch. | Inventory every evidence producer and consumer; convert each to the exact access-observation reference, an explicit typed output/fact slot, or delete it. |
| Canonical expansion can occur once before final node identities are frozen. | Child-operation composition may currently lower graphs in several stages. | Injected nodes could be duplicated or acquire unstable identities, changing graph hashes and effect keys. | Freeze expansion/identity ordering and golden-test direct, nested-child, fan-out, and fan-in construction while certification independently revalidates the result. |
| Operational telemetry remains non-semantic. | Some deployments may require durable compliance evidence or guaranteed delivery to an external sink. | Treating such delivery as a best-effort hook would overstate the guarantee; treating ordinary telemetry as states would bloat and couple semantics. | Derive ordinary telemetry from journal/driver observations without authority. Model compliance evidence that affects decisions as explicit typed states, and guaranteed external delivery as a qualified effect. |
| Compiled Rust registration is sufficient for extensibility. | Future consumers may request WASM, dynamic-library, or remote state implementations. | The proposed state catalog does not define a stable ABI or isolation boundary for them. | Keep this cutover to compiled, certified registrations. Design dynamic execution as a separate authority and isolation contract. |
| Selected compiled planner, state, and capability implementations are trusted to obey their certified purity, determinism, audited-IO, and secrecy contracts. | Rust signatures do not prevent compiled code from reading time, environment, filesystem, network, globals, RNG, or FFI directly. | Re-expansion or replay may detect a differing result but cannot prove that a matching result had no undeclared influence; a capability could bypass the audited wrapper internally. | Treat registrations as reviewed trusted platform code, add conformance and adversarial tests, and restrict direct transport/ambient-authority dependencies. If untrusted implementations are required, introduce a capability-confined sandbox in a separate design. |
| Full transition and artifact retention is acceptable initially. | Complete future analysis requires the referenced bytes, not only their hashes. | Indefinite retention can produce material storage growth. | Measure expected volume. Design garbage collection only after a complete dependency-closure and archival contract exists. |
| A same-journal history scan is sufficient for the initial reserved fact-selection capability. | Fact volume and latency objectives are not defined. | Cross-run selection can become unbounded, and its two audit appends may be material. | Locate every fact-bearing transition through a validated routing column and batch-verify it initially. Measure full audited requests, and add only a rebuildable, watermarked candidate index when required. |
| Portable replay can carry the transitive proof closure of cross-run root sources. | A correction chain may reference several earlier closed runs and profiles. | Bundles may grow materially, while omitting one source would make effective-output or evidence-role verification incomplete. | Require an acyclic closed-source dependency DAG, deduplicate proof objects, measure bundle size, and reject verified replay when any dependency proof is absent. |
| Cross-run consumers can author queries from base inputs, and same-run fact flow can use graph edges. | Existing code may derive a second query adaptively from selected facts or use the fact store as indirect same-run wiring. | Hidden multi-phase reads or compatibility materializers would return. | Inventory consumers; split adaptive queries into typed read-state chains and replace same-run queries with explicit typed edges. |
| Generic typed read-response materialization can resolve selected fact value references. | Existing materialization may be limited to state inputs. | A fact-specific loader would create another runtime path. | Generalize the ordinary content-addressed response materializer and prohibit a fact-only loader. |
| The canonical semantic-state digest schema can be frozen independently of Rust layout. | The RFC defines its components but not final byte-level versioned schemas. | An implementation refactor could invalidate history or Postgres/memory parity. | Specify canonical test vectors for genesis and every transition variant before implementation cutover. |
| All semantic live probes can move after `RunAdmitted`. | Current launch or routing paths may probe providers before a run exists. | The platform-wide audit claim would have an unjournaled prefix. | Inventory admission paths and model each necessary probe as a bootstrap read state; otherwise explicitly narrow the product claim before acceptance. |
| Every logical start has a stable non-secret invocation identity. | Some callers may currently rely on server-generated run IDs and retry by starting again. | Admission acknowledgement loss could create another run and therefore another effect-key namespace for the same business request. | Require a caller-supplied or deterministically derived invocation identity at every admission API and test retry/attach across transport ambiguity. |
| EVM can eventually be placed behind a durable keyed executor. | No wallet or relayer currently owns cross-process nonce selection, signing, rebroadcast, and terminal evidence. | EVM writes remain unavailable after the core cutover. | Keep EVM mutation unregistered until a reviewed executor and conformance suite qualify. |
| Existing histories may be rejected. | MFM is pre-production, but deployments may contain useful evidence. | New binaries will not read old runs. | Inventory and export required evidence first, then reset the schema with explicit legacy rejection and no compatibility reader. |

## Terminology

**Semantic transition**
: One append-only state-machine change verified against the certified graph, exact inputs, accepted
evidence, and previous journal head.

**Transition record**
: The canonical before/input/execution/result/after representation of one semantic transition.

**Commit batch**
: One atomic store append having exactly one legal purpose: run admission, one semantic transition
with optional structural closure, one access authorization, or one access observation. Each purpose
may bind its required or newly admitted immutable objects.

**Transition input manifest**
: The exact typed bindings consumed by a state, including source lineage, config, context, selected
same-run facts, schema identities, and content-addressed value evidence. Cross-run selections are
committed read observations, not pre-materialized inputs.

**State execution contract**
: The certified closed choice of pure, external-read, or recoverable-effect callbacks for one state
descriptor, including its schemas, capability or executor identity, canonicalizers, failure policy,
and live/replay verifier identity.

**State frame**
: One typed, immutable execution view over the deterministically prepared input-manifest candidate,
decoded config, input tree, and certified context for one node occurrence. It is not append
authority by itself.

**External access**
: One independently meaningful application-protocol operation invoked through a certified live
read capability or keyed effect executor at the MFM-controlled capability boundary. Protocol
encoding, local validation, and response decoding belong to the same access; another HTTP/RPC
method, independent batch member, adaptive request, retry, redirect, failover, or provider
reselection is another access.

**External access authorization**
: A durable record that MFM authorized zero or one independently meaningful application-protocol
operation through the identified capability with the identified public request.

**External access observation**
: A durable record of the reviewed result or safe failure returned to MFM for one authorization.

**Crash-ambiguous access**
: An authorization without an observation. The affine authority may have been unused, the call may
be in flight, or a result may have been lost before persistence.

**Representable result**
: A bounded, reviewed, typed, non-secret result admitted under the certified capability contract.

**Unrepresentable result**
: A returned response that cannot be admitted under the reviewed result schema. Only a safe failure
classification and explicitly approved metadata may be retained.

**Effect request**
: The immutable semantic request and stable key produced by an effect state before any
mutation-capable access.

**Keyed ensure**
: The certified executor operation that may be called repeatedly with one committed effect key and
request digest and must converge on one logical external effect.

**Derived view**
: A transient fold over the journal and certified spec. It is never independent semantic authority.

**Candidate index**
: Rebuildable data used only to narrow a query. Returned candidates are rehydrated and verified from
the journal and objects.

**Framework plan expansion**
: A deterministic, hash-defining planning step that may surround eligible authored nodes with
ordinary typed framework states before certification. It is never a runtime callback or
operation-author convention.

**Run access authority**
: A transient, opaque, purpose-bound authority minted by the app authorization boundary for one
tenant, store, and run or admission candidate. It is never persisted and is not semantic state.

## End-State Authority Model

```text
certified typed graph
        +
append-only run journal
        +
content-addressed objects
        |
        +-- scheduling and resume
        +-- recovery
        +-- replay
        +-- transition before/after trace
        +-- status and public output
        +-- fact selection
        +-- lineage and future analysis
```

The irreducible logical persistence is:

```text
RunJournalStore
  journal commits
  journal records
  immutable content-addressed objects
  journal-to-object bindings
  store identity
  store-wide commit ordering
```

Several physical tables can implement this one logical store. Separate tables for journal
envelopes, records, blobs, and bindings do not create competing semantic models when they are
admitted and verified through one transaction and one API.

Current configuration for future runs, secret storage, executor delivery state, operational
telemetry, and optional client cursor state remain separately owned concerns. Exact configuration
and capability bindings selected for an admitted run are retained in that run's `RunAdmitted` root
record.

## Journal Record Algebra

The proposed top-level record algebra is deliberately small:

```text
RunJournalRecord ::=
    RunAdmitted
  | StateTransitionCommitted
  | ExternalAccessAuthorized
  | ExternalAccessObserved
  | RunClosed
```

The records have different authority:

| Record | Authority |
| --- | --- |
| `RunAdmitted` | Establishes the certified run root, spec, config, seeds, identity, and initial state. |
| `StateTransitionCommitted` | Changes typed node/run state and records complete input/output lineage. |
| `ExternalAccessAuthorized` | Authorizes zero or one audited application-protocol operation; changes no semantic state. |
| `ExternalAccessObserved` | Records a reviewed capability return or safe failure; changes no semantic state. |
| `RunClosed` | Structurally seals semantic state at a verified terminal transition. |

Facts, typed bindings and outputs, public output, effect requests, and typed failures are fields or
referenced values of `StateTransitionCommitted`, not independent lifecycle state machines.

## Run Root Record

The first commit retains the complete root lineage:

```text
RunAdmitted {
    version,
    run_id,
    tenant_scope_id,
    invocation_identity,
    entry_point_operation_id,
    operation_contract_ref,
    spec_hash,
    certified_spec_ref,
    certificate_ref,
    state_implementation_manifest_ref,
    capability_binding_manifest_ref,
    config_manifest_ref,
    seed_manifest_ref,
    context_manifest_ref,
    cross_run_source_manifest_ref,
    initial_bindings,
    genesis_digest,
    initial_run_state_digest,
}
```

Run identity is canonical:

```text
run_id = H(
    "mfm.run-id.v1",
    store_scope_id,
    tenant_scope_id,
    entry_point_operation_id,          # stable entry-point namespace, not contract version
    invocation_identity,
)
```

`invocation_identity` is a canonical non-secret logical-start idempotency key. The store permits
one admission for
`(store_scope_id, tenant_scope_id, entry_point_operation_id, invocation_identity)`. Repeating the
same identity with the exact root candidate reloads/attaches to the existing run; changing spec,
contract, config, seeds, context, or certificate conflicts rather than creating another run.
`tenant_scope_id` is an immutable non-secret ownership scope, not a user credential or mutable
membership list. `store_scope_id` is a never-reused lineage namespace: a destructive reset must
generate a fresh scope and epoch before it can admit another run.

A lost admission acknowledgement is resolved by the same derived `run_id` and
`append_request_id`. It never generates a fresh run identity. This is required because a fresh run
would also derive fresh effect keys and could duplicate a business mutation despite perfect
per-effect executor convergence.

If the original append identity was lost with the process, `AdmitRun` still looks up the derived
run/admission logical key. Exact root content returns `ExistingSame` and attaches without another
commit; different root content returns `Conflict`. Only a directly observed first commit returns
`NewlyAdmitted`, and connection ambiguity returns `OutcomeUnknown`.

Every admission and later store-backed run surface requires an opaque `RunAccessAuthority` with
one closed purpose:

```text
RunAccessGrant =
    Admit
  | Drive
  | Replay
  | ReadPublic
  | InspectTrace
  | InspectAudit
  | Export
```

For `Admit`, it binds the exact store scope, tenant scope, entry-point operation, and invocation
identity; the candidate `tenant_scope_id` must match. After admission it binds the exact store
scope, tenant scope, run id, and grant. The app authenticates the caller and mints this sealed,
non-serializable authority; kernel/store entry points validate its binding without consulting a
mutable policy oracle. Tokens, principals, tenant membership, ACL snapshots, and credentials are
not journal fields. Revocation refuses future minting. If administrative access-decision auditing
is required, it belongs to a separate security-audit facility, not the semantic run journal or
external-capability audit algebra.

API notation may refine the sealed authority by its grant, such as
`RunAccessAuthority<Drive>`. The app owns authentication, authorization policy, and minting; the
kernel and store own only type/binding validation and never independently broaden a grant.

An offline verifier may inspect a bundle already produced through an authorized `Export` without a
live token; it has no store/object dereference authority beyond the supplied bytes. `Replay` gates
loading authoritative store-backed history, not pure verification of caller-held data.

The manifests enumerate every typed root slot, schema and semantic type, content digest, object
evidence, context constraint, and exact state implementation or capability binding selected for
this graph. They contain only entries required by the certified spec. `initial_bindings` are the
exact typed values available before the first state transition. Any cross-run root binding must use
the spec-resolved effective-output or evidence-only source role; copying a raw object reference
cannot bypass source lineage or framework post-gating.

`capability_binding_manifest_ref` addresses a canonical manifest of content-addressed bindings:

```text
CapabilityBinding =
    ReadCapabilityBinding {
        capability_contract_ref,
        admitted_implementation_ref,
        safe_classifier_contract_ref,
        reviewed_source_scope_ref?,
    }
  | ExecutorBinding {
        executor_contract_ref,
        admitted_implementation_ref,
        executor_deployment_ref,
    }

ExecutorDeployment {
    executor_namespace_ref,
    durable_ledger_generation_ref,
    tenant_scope_id,
    evidence_authority_ref,
    resource_ownership_ref,
}

CapabilityBindingRef {
    schema_id,
    content_digest,
}
```

`ReadCapabilityBindingRef` and `ExecutorBindingRef` are typed wrappers over
`CapabilityBindingRef` that verify the referenced variant; they do not introduce another identity.
`executor_deployment_ref` content-addresses the canonical non-secret `ExecutorDeployment` object
shown above.

These are closed public identities, never endpoints, credentials, signer handles, or
secret-derived values. An executor binding is the one immutable recovery-routing identity. Its
reference, rather than a copied subset of its fields, is used by the effect request, effect key,
authorization, executor ledger, terminal evidence, restore/failover routing, and replay. A pending
effect can be driven only through its original binding. An upgrade that cannot load that exact
binding and its non-rolled-back ledger leaves the effect blocked rather than treating it as new.

The executor contract defines keyed `ensure`, downstream convergence, the executor safe-failure
classifier, delivery-audit and terminal-evidence schemas, canonicalization, pure verification, and
finite per-effect delivery-record and retained-frontier byte bounds, behavior at bound exhaustion,
and the required evidence and resource-ownership properties. `executor_deployment_ref` fixes the
concrete ledger namespace and generation, tenant scope, proof/attestation authority or trusted
observer, and owner of nonce, UTXO, sequence, inventory, or other cross-effect coordination.
Admission requires its tenant scope to equal `RunAdmitted.tenant_scope_id` and rejects any
incomplete binding. Runtime, replay, effect identity, and evidence propagate and compare only the
one `ExecutorBindingRef`; they do not copy its contract or deployment components. Read bindings
have no executor deployment.

`certificate_ref` resolves a certificate that binds the published entry-point contract, retained
canonical authored-program artifact, exact `PlanningProfile`, planner contract, and expanded
`spec_hash`. Admission reruns that pure planner and requires the expanded graph to match before the
certificate can authorize a run.

The canonical root source manifest contains:

```text
CrossRunSourceManifest {
    bindings: [
        {
            root_field_path,
            source: CrossRunSourceRef,
        }
    ],
}

CrossRunSourceRef =
    EffectiveOutputSource {
        source_store_scope_id,
        source_store_epoch,
        source_admission_ref,
        source_run_id,
        source_spec_hash,
        source_node_id,
        effective_transition_ref,
        effective_output_ref,
        source_closure_ref,
    }
  | EvidenceOnlySource {
        source_store_scope_id,
        source_store_epoch,
        source_admission_ref,
        source_run_id,
        source_spec_hash,
        source_node_id,
        raw_transition_ref,
        raw_result_or_evidence_ref,
        certified_evidence_role_ref,
        source_closure_ref,
    }
```

`EffectiveOutputSource` resolves the logical source through the source certified expanded graph.
For a wrapped occurrence it reaches the final effective post output; for an unwrapped occurrence
the authored output is already effective.
`EvidenceOnlySource` is legal only for a destination slot whose certified contract explicitly
accepts that correction-evidence role. It cannot satisfy an ordinary domain value, output, fact,
public output, or equivalence claim. Producer-bound semantic types cannot be supplied as anonymous
config/context merely by copying identical bytes. Every source run is closed before destination
admission, making the transitive source dependency graph acyclic. In this baseline,
`source_store_scope_id` and `source_store_epoch` must equal the destination store, and the resolved
source admission's `tenant_scope_id` must equal the destination run tenant. Cross-store and
cross-tenant source authority is rejected rather than fetched through ambient IO or inferred from
possession of a reference. The destination's `Admit` authority permits only the private
admission-time verification of same-store, same-tenant sources required by that exact candidate; it
does not grant the caller an independent source trace, audit, export, or object-reading surface.
The baseline therefore needs no second source-consumption grant.

Objects first produced by admission use `ThisAdmission(slot)` or logical seed/config identities,
never the future `RunAdmitted` record coordinate. `certified_spec_ref` is the retained canonical
spec object whose content digest equals `spec_hash`.

Admission verifies the certificate, invocation identity, operation/spec relationship, exact
selected state-implementation and capability-binding manifests, every cross-run source
closure/lineage/role/effective-output resolution, all object bytes/evidence, the explicit genesis
predecessor, and the canonical initial semantic-state digest. Adding an unrelated entry to a
process catalog cannot change an existing run. The first transition can therefore prove every root
input without consulting current configuration or deployment state.

## Complete State Transition Record

The normative logical transition schema is:

```text
StateTransitionCommitted {
    version,
    spec_hash,
    node_id,
    state_contract_ref,

    before: {
        journal_head,
        run_state_digest,
        run_phase,
        node_phase,
    },

    body:
        PureSettled {
            input_manifest_ref,
            settlement,
        }
      | ReadSettled {
            input_manifest_ref,
            request_ref,
            consumed_observation_ref,
            settlement,
        }
      | EffectRequested {
            input_manifest_ref,
            effect_key,
            semantic_request_ref,
            request_digest,
            executor_binding_ref,
        }
      | EffectSettled {
            request_transition_ref,
            request_input_manifest_ref,
            consumed_terminal_observation_ref,
            settlement,
        }
      | DependencySkipped {
            blocking_sources,
        },

    after: {
        run_state_digest,
        run_phase,
        node_phase,
        binding_delta,
    },
}
```

The body is a sum, not an independently selectable cause/outcome product. It cannot encode a pure
effect request, a second request during settlement, or success fields on a skipped transition.

Its logical identity is `(run_id, node_id, slot)`, where `node_id` is the certified node-occurrence
identity and `slot` is
`Request` only for `EffectRequested` and `Settlement` for every settled or skipped variant. The
store permits at most one record in each legal slot, requires `EffectSettled` to reference the one
request slot, and rejects a second or mismatched settlement.

### Canonical node-occurrence identity

`node_id` has exactly one meaning throughout the expanded spec, transition journal, effect key,
output/fact lineage, cross-run source reference, replay, and trace API: the identity of one
occurrence in the final certified expanded graph. The planner retains a deterministic mapping from
each authored occurrence to its final expanded occurrence or occurrences; authored drafts do not
carry a second authoritative node identity. There is no separate
`certified_node_occurrence_id`.

The bound planner assigns it after child-operation composition and framework expansion from one
versioned canonical preimage:

```text
NodeIdentityPreimage {
    identity_contract_version,
    canonical_expansion_path,
    state_contract_ref,
}

node_id = H("mfm.node-occurrence.v1", canonical(NodeIdentityPreimage))
```

`canonical_expansion_path` is assigned before node hashing and distinguishes authored, nested-child,
bridge, injected-pre, protected, and injected-post positions without using process order, map
iteration, run identity, store coordinates, or the future spec hash. `state_contract_ref` prevents
a path from silently being retargeted to different state semantics. The exact planner, planning
profile, config, context, graph edges, and static binding descriptors remain bound by the
certificate and `spec_hash`; they do not need to be copied transitively into every node identity.
Every authoritative use of `node_id` is scoped by its admitted run and certified spec. Duplicate
node identities, noncanonical paths, or nondeterministic expansion reject certification.

The byte schema, path grammar, enum tags, absent/empty encoding, and direct, child, fan-out, fan-in,
and injected-node golden vectors are part of the schema-freeze gate. The final implementation may
reuse the existing `NodeId` type; it must not introduce another occurrence identity wrapper with
independent derivation.

Body legality is closed:

| Certified execution case and derived condition | Legal body |
| --- | --- |
| Pure, `Ready(Unstarted)` | `PureSettled` |
| Read, `Ready(Unstarted)` | `ReadSettled` |
| Effect, `Ready(Unstarted)` | `EffectRequested` |
| Any case, `Unstarted` and certified skip-eligible | `DependencySkipped` |
| Effect, `AwaitingEffect` | `EffectSettled` only |

An awaiting effect cannot fail or skip through another variant. It must remain pending until
terminal effect evidence justifies `EffectSettled`.

`input_manifest_ref` addresses one canonical immutable object:

```text
InputManifestRef {
    artifact_id,
    content_digest,
    evidence_hash,
    schema_id,
}

InputManifestObject {
    input_schema_id,
    context_ref,
    bindings: [
        {
            field_path,
            source:
                RunAdmission { record_ref, field_ref }
              | TransitionOutput { this_run_id, transition_ref, output_ref }
              | TransitionFact { this_run_id, transition_ref, fact_ref }
              | Context { context_ref, field_ref }
              | Config { config_ref, field_ref },
            value_ref,
        }
    ],
}
```

There is no independently serialized inline copy. A pure or effect-request transition admits and
references the object in its own commit. The first read authorization atomically admits and
references the input manifest and its one state-authored request before access; later
authorizations and `ReadSettled` reuse those exact references. Trace readers resolve each once.
`EffectSettled.request_input_manifest_ref` must equal the reference in its request transition.
Before that admitting commit, `StateFrame` is only a verified view over this deterministic
manifest candidate; possession of the frame or staged object cannot mint journal authority.

The input-tree contract versions the field-path grammar. Paths are unique and ordered by the
certified schema, not map iteration. Optional absence, empty collections, fan-in order, and union
discriminants are represented explicitly. Validation proves completeness against
`input_schema_id`; duplicate, unknown, omitted required, or differently ordered slots reject.
`Config` and `Context` sources ultimately reference immutable objects in `RunAdmitted`, never
mutable current configuration. Direct `TransitionOutput` and `TransitionFact` bindings are same-run
certified graph edges only. Every cross-run value enters through a hash-bound
`RunAdmitted.cross_run_source_manifest_ref`; cross-run fact selection enters through the ordinary
audited read protocol defined below. Neither can appear through later pre-execution input
materialization.

`settlement` is:

```text
Succeeded {
    output_bindings,
    fact_emissions,
}
|
Failed {
    typed_failure_ref,
}
```

A failed node produces no consumable output or fact. A domain value such as `Result<T, E>` is a
successful typed output, not a failed node with an output binding. `DependencySkipped` is a
persisted transition and names the complete set of terminal producers that can no longer provide
its required inputs, under the node's certified dependency contract. It produces no output or
fact. A run result cannot be a skip source because it is derived only after every occurrence is
terminal.

There is no variable-cardinality generic `evidence_refs` bag. Read evidence is the exact
`ReadSettled.consumed_observation_ref`; effect evidence is reachable only through
`EffectSettled.consumed_terminal_observation_ref`. Any other domain evidence is an explicitly
declared typed output or fact slot with certified cardinality, schema, role, and lineage. Required
safety evidence is never optional. Operational telemetry is not transition evidence.

`EffectSettled` does not resolve config, facts, context, or state inputs again. It reuses the exact
request transition and input-manifest reference, then adds one committed terminal observation and
its result-bound evidence. It does not repeat an independently authoritative evidence reference;
the terminal observation is the only route to the `TerminalEffectEvidence` envelope. The exact
Rust representation uses sealed variant-specific types rather than one public bag. The persisted
canonical representation nevertheless exposes the complete reviewed trace above.

`state_contract_ref` binds the state/descriptor version, closed execution protocol, input and output
schemas, read/effect request author, pure reducer or verifier, canonicalizers, required executable
identity, required capability or executor contract, and certified failure policy. The
per-run manifest selects one concrete capability or executor binding satisfying that requirement;
the reusable state contract does not contain tenant/deployment binding. Replay verifies each body
variant under both exact identities and the graph-owned dependency contract.

### Before and after

The state-local before view is the exact transition input manifest plus the exact committed request
or observation required by its execution case, or the complete blocking-source and skip proof for
`DependencySkipped`. The state-local after view is the typed result, outputs, facts, and evidence.

The run-wide before view is the verified fold at `before.journal_head`. The run-wide after view is
the deterministic result of applying the transition.

MFM does not persist a complete universal run snapshot before and after every transition. That
would duplicate all prior state, amplify writes, and recreate a second model. The transition keeps
human-inspectable phases and a binding delta together with before/after state digests. Any full
snapshot is derived by folding the journal.

### Exact input lineage

Every input-manifest binding must identify:

- its named position in the certified input tree;
- seed, config, prior output, same-run fact, or context origin;
- producing transition where applicable;
- schema and semantic type identity;
- context constraint;
- content digest;
- retained object evidence.

For cross-run fact selection, the consumed authorization/observation chain records the canonical
query, authorization-order frontier, exact selected fact and producer identities, object evidence,
and explicit empty results. The read transition then records which observation it accepted.

Input bytes are not duplicated into every transition. They are stored once as immutable
content-addressed objects and referenced by the manifest.

Replay rejects when the admitted implementation cannot reproduce a state result from the certified
input manifest and accepted evidence. This is conditional on the selected compiled implementation
obeying its no-ambient-authority contract: replay cannot prove that a coincidentally matching result
did not consult an undeclared clock, environment value, global, RNG, filesystem, network, or FFI.

### Results, outputs, facts, and errors

Inside the candidate, successful outputs use relative `ThisTransition(output_ordinal)` bindings
with schema, semantic type, context, lineage, content digest, and object evidence. After append,
their stable `OutputRef` is derived from the containing `TransitionRef` and ordinal.

Facts are first-class transition emissions. Inside the candidate each carries a local emission
ordinal, claim identity, descriptor, response object, subject/query material, and provenance. Its
producer and `StoreCommitOrder` are implicit in the containing transition and commit; `FactRef` is
derived afterward from `TransitionRef` and emission ordinal. Neither output nor fact payload embeds
its own future record hash or store-assigned coordinate.

A typed terminal failure carries reviewed redacted error evidence and produces no consumable
output or fact. Runtime or provider raw errors never cross this boundary.

Public output is a direct certified typed binding, or the typed output of an ordinary pure state
when a real semantic projection is required. Transport text/JSON rendering is not completion
authority and cannot change the semantic value.

## Commit Batch Contract

Atomicity belongs to the commit batch; traceability belongs to each record.

The legal shapes are exhaustive:

```text
run admission:
  certified spec, certificate, authored-program, exact-planning-profile, planner-contract,
  state-implementation-manifest, capability-binding-manifest, config, seed, context,
  cross-run-source-manifest, and source-proof object bindings
  + RunAdmitted

pure, read, or dependency-skip settlement:
  required and admit-or-verify variant-required input, evidence, output, or failure object bindings
  + StateTransitionCommitted(PureSettled | ReadSettled | DependencySkipped)
  + RunClosed?                         # when this transition terminalizes the run

external-access authorization:
  required and admit-or-verify input-manifest and request object bindings
  + ExternalAccessAuthorized

external-access observation:
  required and admit-or-verify reviewed result or safe-failure object bindings
  + ExternalAccessObserved

effect request:
  required and admit-or-verify input-manifest and semantic-request object bindings
  + StateTransitionCommitted(EffectRequested)

effect settlement:
  required and admit-or-verify evidence/output/failure object bindings
  + StateTransitionCommitted(EffectSettled)
  + RunClosed?                         # when this transition terminalizes the run
```

`effect request` and `effect settlement` are typed cases of the one semantic-transition shape, not
additional batch purposes. There is exactly one top-level record except when `RunClosed` follows
the terminal transition. A batch never combines an authorization or observation with a semantic
transition, another audit record, admission, or closure. Observation therefore commits before the
pure reducer constructs a consuming transition.

The admission batch has an explicit genesis predecessor, establishes the deterministic initial
state digest, and contains no audit or closure record. `RunClosed` is legal only because closure is
an inseparable structural consequence of the transition immediately before it.

The store publishes a new journal head only after validating and applying the whole batch. Readers
never observe a partial batch. Incremental folds apply the entire batch or none of it.

### Journal heads, record references, and state digests

The physical predecessor contract is:

```text
JournalPredecessor =
    Genesis {
        store_scope_id,
        store_epoch,
        run_id,
        genesis_digest,
    }
  | JournalHead {
        run_sequence,
        commit_digest,
        store_commit_order,
    }
```

`genesis_digest` is the domain-separated canonical hash of the other genesis fields. It is legal
only for `RunAdmitted`, at run sequence one, when that run identity does not exist. Every later
commit names the exact current `JournalHead`. An intervening audit-only append therefore makes a
prepared transition stale; runtime reloads the one journal and reconstructs the pure candidate
without repeating a retained external access.

An exact record reference is:

```text
RecordRef {
    run_id,
    run_sequence,
    ordinal,
    record_hash,
}
```

Typed references such as `TransitionRef`, `AuthorizationRef`, and `ClosureRef` wrap `RecordRef` and
verify the referenced schema and logical identity.

`SemanticHead` is the derived reference to the latest `RunAdmitted` or
`StateTransitionCommitted` record plus its containing commit digest. Audit records advance
`JournalHead` but not `SemanticHead`. The baseline deliberately uses the exact physical
`JournalHead` for append compare-and-swap anyway: one predecessor rule is smaller than a second
semantic-CAS protocol, and an audit interleaving requires only pure reconstruction from retained
observations.

`run_state_digest` is not a hash of a Rust fold struct. It is a versioned,
domain-separated hash of a canonical semantic state:

- the certified spec hash and digest-contract version;
- run phase and ordered certified node phases;
- committed typed binding positions, logical output/fact identities, schemas, and content digests;
- unresolved effect node slots, keys, request digests, and executor bindings; and
- public-output logical identity/content digest and terminal-result variant where present.

The canonical contract uses spec order, explicit variants, sorted maps where the spec does not
supply order, and no floats. It excludes audit records, physical journal coordinates, worker
identity, and operational timestamps. `RunAdmitted` establishes the deterministic initial digest;
every semantic transition records and rederives the next digest. Audit-only commits advance
`JournalHead` but leave the state digest unchanged.

Within this digest, output and fact logical identities use certified node occurrence plus local
ordinal and content identity, not the coordinate-bearing `OutputRef` or `FactRef` used to load the
record.

The human-readable phases and `binding_delta` stored in a transition are checked redundant
assertions over this canonical state, never another authority. The transition's containing commit
derives its after-journal-head only after append; that future coordinate is not part of the
transition payload or semantic-state digest.

### Commit envelope and hash contract

The commit envelope binds:

```text
CommitEnvelope {
    store_scope_id,
    run_id,
    run_sequence,
    predecessor,
    append_request_id,
    candidate_digest,
    commit_digest,
    store_commit_order,
    ordered_record_hashes,
    ordered_object_bindings,
    artifact_admission_intents,
    committed_at,                      # operational, excluded from semantic hashes
}
```

`append_request_id` is a caller-generated, collision-resistant, non-secret opaque identifier for
one intended append. It is not derived from request content. The caller reuses it only while
resolving that append's acknowledgement. `candidate_digest` binds the exact
expected predecessor, batch purpose, ordered candidate record envelopes, and canonical
object-path bindings and admission intents before store-assigned coordinates exist.

The store enforces one row per `(store_scope_id, run_id, append_request_id)`. A repeat with the same
predecessor and candidate digest returns the existing commit. Reuse with different content rejects.
Two intentionally distinct appends containing the same public request use different append request
identities. In particular, resolving an ambiguous authorization append never accidentally turns a
later identical authorization into the old record.

Each candidate `record_hash` domain-separately binds its within-batch ordinal, schema, logical key,
canonical payload, and payload-derived `emits_facts` value. It explicitly excludes assigned run
sequence, store order, record ID, and other commit coordinates. The terminal transition hash is
therefore available for the `RunClosed` payload before either record receives coordinates.

After assigning coordinates, the store derives:

```text
record_id = H(
    "mfm.journal-record-id.v1",
    store_scope_id,
    run_id,
    assigned_run_sequence,
    ordinal,
    record_hash,
)
```

Object path binding and admission intent are separate:

```text
ObjectPathBinding {
    record_ordinal,
    field_path,
    authority_use: Preexisting | ProducedHere,
    artifact_id,
    content_digest,
    evidence_hash,
}

ArtifactAdmissionIntent {
    artifact_id,
    content_digest,
    evidence_hash,
    mode: RequireExisting | AdmitOrVerifyExact,
}
```

Path bindings cover existing inputs, same-run facts, selected cross-run fact values referenced by a
committed response, and new outputs/evidence. `authority_use` is derived and validated from the
closed record/field schema; it does not report whether a database insertion happened.

Admission intents are deduplicated by the complete artifact/evidence identity. If any path uses
`Preexisting`, the one canonical mode is `RequireExisting`, and the authority must exist before
this commit begins; a `ProducedHere` path elsewhere in the same batch cannot self-admit a missing
input or prior evidence. Only an artifact used exclusively through `ProducedHere` paths may use
`AdmitOrVerifyExact`.

`AdmitOrVerifyExact` succeeds whether this transaction inserts the immutable admission or a
concurrent transaction already admitted the exact same identity; different bytes/evidence
conflict. `RequireExisting` never creates authority, and its success cannot depend on another
admission intent in the same batch.

The distinct artifact/evidence identity keys in path bindings and admission intents must be equal:
exactly one canonical intent per bound identity, with no missing or unreferenced intent. The
normalized `commit_artifact_bindings` representation persists path usage and canonical intent mode
so either backend can recompute candidate and commit digests after restart.

Both sets are sorted by complete canonical encoding before hashing. A normalized binding table is
only foreign-key support; every row is derived from and checked against these hash-bound sets.

`commit_digest` domain-separately binds every envelope field except itself and `committed_at`,
including append identity, candidate digest, predecessor, assigned sequence and store order,
ordered record hashes, object-path bindings, and admission intents. A loaded journal rejects broken predecessor
linkage, missing or reordered records, routing mismatches, missing or extra bindings, object
evidence mismatches, or bytes that fail content-address verification.

### Sealed append authority

There is no public raw-record append:

```text
PreparedJournalAppend =
    AdmitRun
  | CommitTransition
  | AuthorizeExternalAccess
  | ObserveExternalAccess
```

- Run admission constructs `AdmitRun` from one verified certificate, invocation, and exact root
  objects.
- Runtime constructs `CommitTransition` only from one `VerifiedRunView`, pure state
  verification/reduction, and that view's exact `JournalHead`. Closure is a sealed field of the
  terminal transition candidate.
- The audited capability wrapper constructs authorization and observation candidates from closed
  capability-specific types. Only the store's positive authorization result can add transient live
  access authority.
- Store-owned code alone assigns run sequence, store order, record IDs, routing fields, and commit
  digest.

The store never treats the sealed type as a bypass. Under the run transaction it rechecks append
identity and digest, exact predecessor, legal batch shape and record order, logical uniqueness,
record and object bindings, routing derivation, closure and post-closure rules, effect-request
state, and whole-batch fold legality. The runtime's opaque semantic-verification proof is bound to
the exact candidate and predecessor; semantic interpretation remains runtime-owned while store
structure remains independently enforced.

## External-Access Audit Contract

### What is audited

Every invocation of an MFM-supplied live capability for:

- a read state; or
- a keyed `ensure` call for an effect state

must use this audit protocol.

Low-level journal, artifact, index, and audit-append database calls, runtime configuration loading,
metrics, and telemetry are not individually audited capability calls. Otherwise recording an audit
record would itself require another audit record indefinitely. A state-requested invocation of the
reserved journal-backed fact-selection capability is a semantic read and uses this audit protocol
once at that capability boundary.

An authorization permits zero or one independently meaningful application-protocol operation
entered through an explicit MFM-controlled certified capability API. If boundary entry occurs, an
in-process capability may encode, invoke, and decode exactly that one operation. It must not
initiate another HTTP/RPC method or independent request, retry, redirect, fail over, reselect a
provider, or author a response-dependent operation under the same authorization. Local validation
may instead produce `DidNotEnter`. Each independent or adaptive operation is a separate typed read
state with its own authorization and observation.

The transport envelope is not an escape hatch. In the baseline, one HTTP request containing
several independently meaningful JSON-RPC batch members is several semantic operations and is not
a supported audited capability shape. A collection-valued request is one operation only when it
has one indivisible semantic identity and one shared snapshot, cancellation, and result/failure
contract. Independently invokable, retryable, or outcome-bearing members require separate audit
identities regardless of the method name or HTTP envelope. A future transport-batching design may
combine ready operations only if it retains one authorization identity and one safe outcome
identity per member and proves affine consumption, partial return, cancellation, and replay
ordering. It cannot change the state or audit algebra defined here.

Kernel/TCP retransmission is not another semantic capability entry and cannot be counted from
local Rust typestate. The capability's read or keyed-convergence contract must make such
transport-level duplication safe. An HTTP/RPC library retry that can create another independent
application request is not a transport detail and requires a fresh authorization.

A remote keyed executor may make internal target calls. The MFM journal audits its one
application-protocol call to that executor; it does not mint MFM affine authorities inside the
remote service. Before every independently meaningful executor-to-target operation, the executor
durably appends `DeliveryAttemptAuthorized` under the effect key. Only a positively acknowledged
new record permits zero or one operation. A surviving result adds a linked
`DeliveryAttemptObserved`; a crash between those records leaves an honest unmatched authorization.
Every representable `Pending` or `Terminal` executor result carries a content-addressed candidate
delivery-audit frontier and its complete new suffix. MFM verifies them under the exact executor
binding and atomically retains them with the observation. Replay therefore uses immutable
journal-reachable evidence rather than consulting a mutable executor log. The executor ledger
remains irreducible external-delivery authority, not a second MFM run-state or semantic authority.

Every new direct MFM-controlled live application-protocol operation requires this protocol.
“Obtain audited evidence” is conditional only in the following senses: pure and skipped states
perform no live access, and a read/effect state may reuse sufficient committed evidence instead of
calling again. Once runtime decides to invoke a live capability, durable authorization before entry
and durable observation for every surviving wrapper result are mandatory. There is no unaudited or
best-effort evidence path.

Four distinct rules follow:

1. auditing a new live call is mandatory;
2. persisting its observation is mandatory whenever the wrapper result survives;
3. semantic consumption is conditional because stale, losing, failed, or unmatched observations
   may remain audit-only; and
4. auxiliary domain evidence is optional only when the state schema declares an explicit typed
   optional slot whose presence and absence are canonical.

A generic optional collection of evidence references is forbidden, and required safety evidence
cannot use an optional slot.

Run admission itself performs no external provider, routing, filesystem, signer, or executor probe.
Any live information needed to select or validate a source is modeled as an audited bootstrap read
state after `RunAdmitted`. This keeps the claim “every MFM-controlled semantic external access is
audited” true from the run root and prevents pre-admission IO from influencing an untraceable
configuration choice.

### Production read decomposition

The cutover does not preserve the current aggregate live-reader implementations as hidden
multi-operation capability calls. Their target graphs are ordinary reusable operation
composition:

```text
EVM balance collection:
  audited source/bootstrap read when live validation is required
    -> audited latest-anchor read
    -> ordinary fan-out with one audited token-metadata or balance node per invocation
       at that exact anchor
    -> audited final-anchor confirmation
    -> pure typed aggregation

Bitcoin balance collection:
  audited source/bootstrap read when live validation is required
    -> audited blockchain-info read
    -> audited scantxoutset "start" read with one indivisible collection-valued request
    -> audited block-hash confirmation derived from the scan result
    -> pure typed aggregation
```

Every arrow whose request depends on a prior response is a typed graph edge. Every external method
invocation has its own authorization and observation. Concurrent fan-out is expressed by ordinary
independent nodes in the certified graph; a dropped sibling future cannot disappear inside one
aggregate capability result. The high-level operation builder owns this reusable topology, so
callers do not manually assemble protocol steps and runtime learns no EVM- or Bitcoin-specific
phase.

The production-read prototype gate must exercise partial return, cancellation, compare-and-swap
loss, anchor change, and maximum certified fan-out. Performance may motivate a later
authority-preserving batching design, but cannot relax this audit unit.

The target classifies Bitcoin `scantxoutset "start"` as a read, not a mutation effect, only after
its production capability qualifies that choice. Qualification must establish that it creates no
durable domain mutation; the collection has one indivisible snapshot and outcome; each invocation
has bounded work and retained result; repeated work, provider cost, and shared scan concurrency are
explicitly accepted; and the capability performs only `start`, with no hidden `status`, `abort`,
retry, or provider reselection. Timeout or cancellation after possible entry is `Indeterminate`,
and any later reissue uses a fresh MFM authorization and may execute another full scan after the
earlier work finishes. A reviewed “scan already in progress” return proves only that this invocation
did not start another scan; it does not synthesize the outcome of an earlier authorization. MFM
guarantees separately audited calls and anchor-verified results, not at-most-once scan cost. If the
prototype cannot establish these properties, production Bitcoin collection remains unregistered
until a separate keyed work executor and audited status-read design is specified; the generic
runtime gains no read/effect recovery mode.

### Authorization record

Before constructing live access authority, runtime appends:

```text
ExternalAccessAuthorized {
    semantic_anchor: {
        journal_head,
        run_state_digest,
        node_id,
        node_phase,
    },

    scope:
        Read {
            input_manifest_ref,
        }
      | EnsureEffect {
            effect_request_transition_ref,
        },

    capability_binding_ref,
    capability_operation_id,
    request_ref,
}
```

The record coordinate is the access identity. There is no generic worker `AttemptId`, attempt
number, owner, lease, epoch, or retry counter.

The semantic anchor binds a read to the exact verified state view from which its typed request was
authored, including the initial view where no predecessor state transition exists. An effect
authorization additionally binds the already committed request transition.

For a read, `capability_binding_ref` resolves the exact read contract, admitted implementation,
safe classifier, and optional reviewed source scope selected at admission. For an effect it equals
the exact `ExecutorBindingRef` in the committed effect request. A reviewed source scope is a closed,
non-secret configuration identity, never an endpoint, URL, hostname, filesystem path,
provider-supplied identifier, or arbitrary string.

Under the append transaction, the store requires:

- the anchored journal head and state digest are still current;
- a read node remains ready under the same certified request;
- an effect remains `AwaitingEffect` for the exact request transition, key, and digest;
- capability binding, `capability_operation_id`, request, and executor authority still match the
  certified manifests; and
- the run is not closed.

A stale worker cannot authorize a read for a settled node or an `ensure` call for a settled effect.
Audit records that win the physical-head race force reload; no live access has happened yet.

The certified state request callback reauthors exactly one read request purely from the retained
input manifest under the anchored state view. A `ReadSettled` transition must use that same
manifest and request reference. Request authorship is total over the certified `StateFrame`; input
validation that can fail semantically belongs in an earlier pure state that produces a stronger
typed input. A callback fault blocks execution and is not converted into a semantic failure.

For `Read`, `request_ref` is the canonical typed request authored by the state under
`state_contract_ref`; replay reauthors and compares it. For `EnsureEffect`, it must equal the
immutable semantic request already bound by the referenced transition. It is semantic public
request material, not HTTP/RPC bytes, headers, endpoint selection, or signer payload.

Its normative meaning is:

> MFM durably authorized zero or one independently meaningful application-protocol operation
> through this audited capability boundary using this reviewed public request. The record does not
> prove that boundary entry or remote receipt occurred.

The append has a closed result:

```text
NewlyAppended(
    AuthorizedReadAccess
  | AuthorizedEnsureAccess
)
AlreadyCommitted
Rejected
OutcomeUnknown
```

Only a positively acknowledged `NewlyAppended` result mints a publicly nameable but privately
constructible, non-cloneable, non-serializable, affine authority. The concrete type seals:

```text
authorization_ref
run_id
semantic_anchor
scope
capability_binding_ref
capability_operation_id
request_ref
committed effect request                       # ensure only
```

An idempotently found record, reload, stale response, commit-then-error, or lost acknowledgement
mints no authority. A later physical call requires a fresh authorization append identity even when
the semantic read request or effect request is unchanged.

The audited wrapper consumes and destroys the affine authority when it accepts the invocation,
before local validation or external boundary entry. It never returns that authority. The wrapper
contract permits at most one audited application-protocol operation and forbids retry, redirect,
failover, or pairing the authority with another request. A local rejection after acceptance
produces `DidNotEnter`.

The affine type enforces one entry through the safe MFM wrapper API; it does not inspect trusted
compiled capability code or prove the number of downstream packets or requests. Single-operation,
retry-free behavior is an in-process capability qualification requirement and, where the threat
model requires mechanical confinement, the capability receives only a transport that itself
enforces it. A remote executor's internal calls use the separately labeled mandatory linked
executor attempt evidence defined above.

Only this wrapper can construct the sealed `UncommittedAccessObservation` accepted by
`ObserveExternalAccess`. Schema-valid result bytes, a record reference, or an independently
constructed request cannot mint an observation.

This establishes the one-way audit invariant:

```text
physical MFM-controlled application-protocol operation
    =>
one previously committed ExternalAccessAuthorized
```

The reverse implication is intentionally unsupported.

### Observation record

After the capability returns or the wrapper obtains a safe local failure, runtime appends:

```text
ExternalAccessObserved {
    authorization_ref,

    outcome:
        Returned {
            result_ref,
        }
      | DidNotEnter {
            safe_failure,
        }
      | Indeterminate {
            safe_failure,
        },
}
```

There is at most one observation per authorization. An idempotent retry with the same observation
digest returns the existing record. A different second result conflicts.

While a process still holds a sealed outcome, appending that observation is mandatory and retried
idempotently; an audit-store failure does not authorize another access or permit the outcome to
influence state. “No observation” represents loss of the executing task or process, or unresolved
append ambiguity, not an optional logging policy. Panic, task abortion, cancellation, and forced
shutdown can therefore leave an unmatched authorization even while the process remains alive.

On load, runtime exposes only a sealed `CommittedAccessObservation` that verifies the containing
commit, authorization, run and node, semantic anchor, complete capability binding,
`capability_operation_id`, public request digest, result schema, and observation role. A semantic
transition may reference it only from a strictly later commit.

The outcomes mean:

- `Returned`: the audited capability returned one bounded, reviewed, typed result to runtime. A
  provider-level business rejection is `Returned` when it is represented by the approved result
  schema.
- `DidNotEnter`: the audited wrapper can prove that it rejected or failed before external boundary
  entry.
- `Indeterminate`: entry occurred or may have occurred, but no trustworthy typed result was
  obtained.

`Indeterminate` includes transport interruption, timeout after possible entry, cancellation after
possible entry, malformed or unrepresentable response data, and a response that cannot safely be
converted to the reviewed persisted schema.

An authorization without an observation is `CrashAmbiguous`. Recovery never appends
`Interrupted`, `Abandoned`, or `NotCalled` for it because a successor cannot prove those claims.

### Representable and unrepresentable returns

A representable return is decoded directly into the bounded result type certified for the
capability. The approved result object can be retained and later consumed by a semantic
transition.

An unrepresentable response cannot become a domain value. Runtime records:

- a stable safe failure code;
- `failure_class = unrepresentable_response`;
- the boundary stage;
- an optional reviewed coarse size class; and
- an optional redacted diagnostic object.

MFM does not persist the raw provider body first and sanitize it later. It decodes directly into the
narrow retained result or emits a safe failure. It never fingerprints rejected raw bytes: a
high-entropy value may still be a credential or bearer token. Hashing unsafe material is not an
acceptable substitute for redaction.

Unrepresentable bytes cannot become a domain result. A certified safe-failure classification may
justify a typed node failure under the state contract's pure failure policy. Because the raw bytes
are deliberately absent, replay verifies the audited wrapper's sealed attestation under the
admitted implementation and classifier contract; it does not claim to re-parse the provider
response.

### Observation before state reduction

The read-side capability shape is likewise bound:

```text
trait AuditedReadCapability {
    type Request;
    type Response;

    async fn read(
        access: AuthorizedReadAccess<CommittedRequest<Read, Request>>,
    ) -> AccessReturn<Response>;
}
```

The runtime result flow is:

```text
AuthorizedReadAccess | AuthorizedEnsureAccess
    -> audited wrapper consumes authority
    -> sealed UncommittedAccessObservation
    -> CommittedAccessObservation
    -> pure state verifier/reducer
    -> StateTransitionCommitted
```

Only a committed observation can influence a semantic transition.

This intentionally adds a durable boundary after a live call and before state reduction. If the
reducer, output validation, or later transition append fails, the external return remains auditable
and recovery can retry pure verification without repeating the access.

The transition records the exact one observation it consumed. Other returned, stale, failed, or
unmatched authorizations remain audit-only history.

For `ReadSettled`, the direct `request_ref` is trace convenience, not another authority. It must
equal the unique request reached through
`consumed_observation_ref -> authorization_ref -> request_ref`; any mismatch rejects.

Observation use is scope-checked:

- `ReadSettled` accepts exactly one observation authorized for that run, node,
  input-manifest reference, capability, `capability_operation_id`, request, and response schema.
- `EffectSettled` accepts only `Returned(Terminal { .. })` for its exact request transition and
  executor.
- `Returned(Pending { .. })`, unmatched authorization, and `CrashAmbiguous` are never settlement
  evidence.
- `DidNotEnter` or `Indeterminate` may be consumed only by an explicitly certified pure read-failure
  policy; retry count, elapsed time, and worker behavior are not policy inputs.
- Each observation can justify at most one committed semantic transition. Reuse after that point is
  through the transition's typed output, not raw audit evidence.

The store rejects cross-run, cross-node, cross-capability, cross-`capability_operation_id`,
cross-request, wrong-schema, stale-authority, and already-consumed substitution.

### Read workflow

```text
Ready node
  -> author one deterministic typed read request
  -> append ExternalAccessAuthorized
  -> audited wrapper consumes AuthorizedReadAccess
  -> perform at most one application-protocol operation
  -> append ExternalAccessObserved
  -> state accepts the committed observation and reduces purely
  -> append StateTransitionCommitted(ReadSettled)
```

Rules:

- Exactly one state-authored request exists for the node occurrence. Every retry reauthors the same
  request and creates a distinct authorization.
- The state contract examines eligible committed observations in journal order. The first
  `Settlement` fixes the exact consumed observation. If every eligible observation returns
  `InsufficientEvidence`, `drive_once` authorizes another call with the same request instead of
  repeatedly retrying a no-op settlement. Runtime arrival time, retry count, and worker policy
  never choose the semantic response.
- `Returned` evidence may settle the state only after the state verifier accepts it.
- `DidNotEnter` and `Indeterminate` may settle only through the certified typed failure policy.
  Unmatched authorizations never produce state output.
- A retry creates a new authorization and a new affine authority.
- Because the certified read contract is non-mutating, crash ambiguity permits another read.
- An operation that marks data read, initiates work, consumes a one-shot token, or creates a
  material cost is not certified as a read unless that cost/retry policy is explicit; a semantic
  mutation uses the keyed-effect path.
- Snapshot-sensitive reads pin their external frontier in the typed request and evidence contract.
- If several observations exist for retries, the winning transition identifies the exact one it
  consumed; every other observation remains audit-only.
- A terminal read failure is a separate semantic transition justified by reviewed failure policy
  and committed audit evidence; an access failure does not automatically terminalize a state.

### Effect workflow

An effect state first commits:

```text
Ready
  -> StateTransitionCommitted(
       body = EffectRequested {
           effect_key,
           semantic_request_ref,
           request_digest,
           executor_binding_ref,
       }
     )
```

Only then may runtime:

```text
AwaitingEffect
  -> append ExternalAccessAuthorized(EnsureEffect)
  -> audited executor wrapper consumes AuthorizedEnsureAccess
  -> call ensure at most once for its bound effect request
  -> append ExternalAccessObserved(
       Returned(Pending { delivery_audit_ref })
       | Returned(Terminal { evidence })
       | DidNotEnter
       | Indeterminate
  )
```

`Pending`, `DidNotEnter`, `Indeterminate`, and unmatched authorizations leave the node
`AwaitingEffect`. A returned pending delivery audit remains immutable audit evidence but cannot
settle the node.

`Returned(Terminal { evidence })` becomes a committed observation. The terminal envelope is the
single owner of its `delivery_audit_ref`. The state-owned pure verifier checks it against the
original inputs, immutable request, certified executor binding, provenance, external identity, and
assurance/finality policy. Only then may a semantic transition settle the node.

Compatible terminal observations are scanned in journal order. The first `Settlement` fixes the
consumed observation. `InvalidEvidence` is an integrity failure and returns `Waiting` without
trying another observation or inventing a domain result. If every terminal observation is
`InsufficientEvidence`, recovery creates a fresh authorization and calls `ensure` again with the
same effect key and request digest. Access-record identity is never used as the effect key.

An audit record never:

- proves application or non-application;
- changes the semantic request;
- authorizes a replacement operation; or
- terminalizes the node.

### Late observations after semantic closure

Semantic run closure and physical journal sealing are different.

After `RunClosed`:

- no new semantic transition is legal;
- no new `ExternalAccessAuthorized` is legal;
- `ExternalAccessObserved` is legal only when it references an unmatched authorization committed
  before closure;
- that observation cannot change the semantic fold, public output, facts, or closure outcome; and
- no second observation for the same authorization is legal.

This permits a stale read or `ensure` invocation to return after another worker settled and closed
the run. Requiring every authorization to acquire an observation before closure would allow one
crashed process to block terminality forever.

The fixed semantic closure coordinate remains part of status and export. The journal head may
advance through constrained audit-only tail records.

### Why this is not the old attempt lifecycle

An external-access authorization:

- exists only because zero or one application-protocol operation may occur;
- contains no worker, owner, lease, epoch, attempt number, retry policy, or interruption;
- does not change node or run state;
- does not block scheduling or terminality;
- does not become effect identity;
- is never taken over, rewritten, or abandoned; and
- pairs with at most one immutable observation.

Pure computation, worker execution, scheduling, and process takeover have no persisted attempt
events. One read or pending-effect evaluation may produce several audit pairs across retries, but
each live application-protocol operation has its own pair. If the state settles, exactly one later
semantic transition names the one observation it accepts; all other pairs remain audit-only.

## Keyed-Convergent Effect Contract

### Core executor shape

The kernel-facing contract is:

```text
trait RecoverableEffectExecutor {
    type Request;
    type Evidence;

    async fn ensure(
        access: AuthorizedEnsureAccess<CommittedEffectRequest<Request>>,
    ) -> AccessReturn<Ensure<Evidence>>;
}

enum Ensure<Evidence> {
    Pending {
        delivery_audit_ref,
    },
    Terminal {
        evidence,
    },
}
```

`delivery_audit_ref` is mandatory for `Pending` and is a required field of terminal `evidence`. It
addresses an immutable, bounded, reviewed, non-secret object retained by MFM through the
observation's `result_ref`; it is never a remote locator. It uses the ordinary content-addressed
`ValueRef` and introduces no independently authoritative handle type. Its logical shape is:

```text
DeliveryAttemptRecord ::=
    DeliveryAttemptAuthorized {
        attempt_ordinal,
        attempt_id,
        target_operation_ref,
    }
  | DeliveryAttemptObserved {
        attempt_id,
        outcome: Returned { safe_result_ref }
               | DidNotEnter { safe_failure }
               | Indeterminate { safe_failure },
    }

DeliveryAuditFrontier {
    executor_binding_ref,
    effect_key,
    request_digest,
    predecessor_frontier_ref?,
    appended_records: [DeliveryAttemptRecord],
    proof,
}

attempt_id = H(
    "mfm.executor-delivery-attempt.v1",
    executor_binding_ref,
    effect_key,
    request_digest,
    attempt_ordinal,
    target_operation_ref,
)
```

Before target boundary entry, the executor durably appends `DeliveryAttemptAuthorized` to the exact
keyed ledger. Only a positively acknowledged new append mints executor-internal authority for zero
or one target operation; reloading an old record or resolving an ambiguous append mints none. A
surviving result durably appends at most one `DeliveryAttemptObserved`. An unmatched authorization
remains ambiguous and is never rewritten to “not called.” A convergence-safe repeat uses a new
ordinal and attempt identity. The canonical empty frontier represents no delivery authorization.

Every `Returned(Pending { .. })` and `Returned(Terminal { .. })` result supplies the complete
content-addressed suffix needed to reach its frontier. MFM verifies the binding, effect key,
request digest, canonical records, predecessor chain, and binding-specific proof, then atomically
admits the new objects with `ExternalAccessObserved`. Accepted frontiers for one effect must form
one prefix chain. An equal frontier is idempotent; an older ancestor is a valid stale concurrent
result but does not regress the private greatest frontier; a descendant advances it. An
incomparable fork, rewritten record, invalid proof, or unrepresentable executor response is
recorded as `Indeterminate` without retaining unsafe bytes or exposing it as verified delivery
evidence. The greatest frontier is a rebuildable private fold over committed observations, not a
persisted executor-status authority.

The executor contract's finite attempt-count and total retained-frontier byte bounds make every
complete new suffix representable in one reviewed result. Before authorizing another target
attempt, the executor proves that the resulting attempt count and complete retained frontier remain
within both bounds. Once either bound is exhausted, or the next authorization could exceed it,
`ensure` may only inspect its own retained state or return already obtained operation evidence. It
cannot enter the target boundary or authorize another target attempt. If no terminal proof can be
obtained, the effect remains permanently pending. The baseline has no delivery-audit pagination
protocol.

This is not a generic evidence bag. The exact bound `executor_contract_ref` fixes both the
delivery-audit and terminal-evidence schemas, canonicalization, verifier, and permitted proof forms;
`executor_deployment_ref` fixes their concrete authority. The executor owns its live convergence
ledger, but after an observation commits, MFM trace and replay never query that mutable ledger to
reinterpret the retained attempt history. An unmatched MFM-to-executor authorization may lack a
result entirely; a later `ensure` result must expose the durable executor history for the effect
key.

Request authorship is pure and non-mutating. It receives only certified config, exact transition
inputs, and context. A request that needs a live quote, nonce, UTXO, or other observed semantic
input must consume the typed output of an explicit earlier read state or leave that choice to the
executor under a certified equivalence constraint.

Volatile delivery choices should normally remain executor-owned within explicit semantic bounds.
If a committed request deliberately pins an expiring quote or resource, it also binds its validity
frontier and a certified terminal-rejection proof. Expiry, timeout, or lookup absence alone cannot
prove permanent non-application and may leave the effect pending.

Only `AuthorizedEnsureAccess`, already bound to a `CommittedEffectRequest` reconstructed from a
verified pending transition, can reach `ensure`. Request and authority cannot be substituted
independently. The raw mutation transport is not supplied to state code or ordinary runtime
execution.

### Effect identity and terminal evidence

The kernel derives, rather than state code inventing:

```text
effect_key = H(
    "mfm.effect-key.v1",
    executor_binding_ref,
    store_scope_id,
    run_id,
    node_id,
)
```

`executor_binding_ref` resolves the one immutable `ExecutorBinding` selected at admission. The
`EffectRequested` record, effect-key derivation, `EnsureEffect` authorization, executor ledger
entry, terminal evidence, restore/failover routing, and replay use that same reference. State-owned
semantic request rules remain in `state_contract_ref`; there is no second ambiguous
`effect_contract_ref` or separately mutable executor tuple.

The key deliberately excludes request bytes. Its one immutable request digest is bound by the
pending transition, and reuse with another digest fails closed. `store_scope_id` remains stable
across a verified non-rollback backup restoration of the same lineage; restoring a resumable store
under a new scope is forbidden. A destructive reset must use a never-before-used
`store_scope_id` and a fresh `store_epoch`, so it cannot recreate a prior run or effect key.
Concurrent clones must share the same executor ledger and deployment fence or only one may drive
effects. The bound executor deployment's namespace, ledger generation, and tenant scope
authenticate the original ledger. Contract upgrades must route an existing pending key back to
that exact non-rolled-back binding; inability to do so leaves the effect blocked rather than
treating the key as new.

Executor evidence is wrapped by a kernel envelope:

```text
TerminalEffectEvidence {
    executor_binding_ref,
    effect_key,
    request_digest,
    delivery_audit_ref,
    external_operation_identity,
    terminal_outcome,
    assurance_policy_ref,
    proof_basis:
        SelfAuthenticatingProof
      | ExecutorAttestation { evidence_authority_ref }
      | TrustedObserver { evidence_authority_ref },
    domain_evidence_ref,
}
```

Replay reports the assurance actually established by `proof_basis`; it does not collapse a trusted
observation, signed executor attestation, and independently verifiable proof into one claim.
It rejects a proof basis, attestation authority, delivery-audit schema/proof, resource domain, or
implementation not admitted by the exact executor contract, deployment, and binding.

### Required convergence law

The certified executor must guarantee:

- the same effect key and request digest always identify the same logical operation;
- the same key with another request or executor binding is rejected;
- repeated, concurrent, and arbitrarily delayed calls converge on at most one semantic external
  effect;
- any nonce, UTXO, signer payload, external id, or transport choice is durably bound before the
  executor crosses its target mutation boundary;
- replacement, if allowed, remains within the fixed certified semantic request;
- terminal evidence remains retrievable after worker and executor restart;
- every independently meaningful executor-to-target operation has a durable authorization in the
  keyed ledger before target entry, permits at most one entry, and has a safe linked outcome when
  that outcome survives; unmatched and indeterminate attempts remain honest;
- every accepted delivery-audit frontier is immutable, binding-specific, and belongs to one
  predecessor-linked prefix chain for the effect key; stale ancestors do not regress its derived
  greatest frontier;
- target-attempt count and the complete retained frontier stay within the finite bound fixed by the
  executor contract; exhaustion cannot discard history or create an unrecorded attempt;
- a fixed terminal operation/outcome may gain append-only evidence strengthening, but never a
  conflicting or weaker outcome;
- a delayed call after MFM settlement cannot create a different mutation;
- pending entries and terminal tombstones remain durable while a delayed call could execute;
- credentials and bearer material never enter MFM's ordinary journal or object surfaces; and
- duplicate delivery costs are either absent or explicitly included in the certified assurance.

The executor contract defines the equivalence relation over every externally meaningful
consequence in scope: primary mutation, nonce/sequence consumption, provider billing, delivery fee,
and independently triggered notification or work. An unavoidable duplicate consequence must be
bounded in the semantic request and reported in terminal evidence. If it is unbounded or outside
the reviewed equivalence relation, the executor does not qualify.

The effect guarantee is:

> Runtime may redeliver calls to a certified keyed executor. Under that executor's recorded
> convergence and resource-ownership assumptions, all calls identify at most one primary domain
> mutation plus only the explicitly bounded delivery consequences, and any terminal settlement is
> replay-verifiable.

Progress is conditional on the executor and destination remaining available and on the qualified
downstream convergence mechanism. Eventual settlement additionally assumes a fair operational
driver continues authorizing `ensure` and the domain can eventually produce terminal evidence.
Permanent pending is the correct safe result when those liveness assumptions fail. Polling,
backoff, and wake policy remain operational and do not create attempt events. This is not generic
exactly-once transport.

### Executor-owned delivery state

An executor ledger alone does not make a non-convergent destination recoverable:

```text
keyed executor ledger entry committed
target mutated
executor crashes before terminal evidence commits
```

After this boundary the ledger cannot safely decide whether to redeliver unless the selected
downstream operation is independently convergence-safe. A certified executor therefore requires
at least one reviewed mechanism:

- permanent destination-native idempotency and lookup;
- a preassigned external identity with authoritative lookup and repeat-safe submission;
- transactional enqueue where the durable queue is the semantic destination; or
- another protocol-specific proof that delayed redelivery cannot create a second semantic or
  cost-bearing effect.

Its durable entry binds effect key, request digest, and the exact executor binding reference.
Before each target operation it appends `DeliveryAttemptAuthorized` and the chosen operation
family; after a surviving result it appends the linked `DeliveryAttemptObserved`. The binding
resolves convergence, evidence, deployment scope, and resource ownership without copying an
independently mutable tuple. Concurrent calls reuse that binding. An unmatched delivery
authorization remains ambiguous; it never authorizes an unsafe different operation. Redelivery
from an ambiguous bound entry is legal only under the recorded downstream convergence proof.
Terminal evidence and its complete delivery-audit frontier persist before return unless the
destination permanently supplies equivalent authority. If MFM rejects insufficient evidence,
later `ensure` calls may return another frontier on the same prefix chain; only a descendant
strengthens evidence for the fixed terminal operation and outcome. They may not rewrite the
operation or contradict earlier evidence, and a claimed descendant may not omit a record from its
predecessor chain.

Replacement candidates, when supported, form an append-only lineage under one fixed semantic
payload and resource identity; a single mutable `chosen_operation_ref` is insufficient. Executor
bindings and terminal tombstones must survive backup restore, failover, migration, stale-replica
promotion, and split-brain attempts without rollback.

This ledger is not a second model of MFM run state. It owns external convergence. The transition
journal owns the requested semantic operation and accepted state result. Collapsing them is safe
only when the destination itself supplies the executor role.

### No weaker recovery mode

The kernel has no `ObserveOnly` versus `RepeatExact` mode and no first-versus-recovery dispatch
permit.

Observation-only recovery is safe containment but not meaningful basic recoverability: a crash
after request commit and before dispatch can wedge forever. Exact transport bytes are also the
wrong generic abstraction. They are insufficient for a non-idempotent destination and unnecessary
for a destination that enforces semantic idempotency by key.

Non-convergent one-shot mutations remain outside certified effect states. A separate product may
expose them as explicitly unrecoverable operations, but they do not weaken this primitive.

## Cross-Effect Coordination Is Executor or Domain Authority

The kernel has no resource lane, serialization key, active-effect table, or journal-derived
external fence.

Per-effect keyed convergence does not prevent two different effect keys from racing for one nonce,
UTXO, account sequence, inventory unit, or business resource. A certified executor or destination
must own that coordination through exclusive resource ownership, one shared durable coordinator,
or atomic domain preconditions. Within one run, the certified state graph may express ordinary
semantic dependencies; it is not a cross-run external lock.

This placement is required for correctness. Releasing an MFM-local key after settlement cannot
revoke an affine authority already issued to a delayed call, so it cannot establish external
quiescence. The executor must make that delayed call converge safely even after later effects
begin.

## Thin Runtime and Typed State Execution

The runtime is a stateless interpreter of one certified graph and one verified journal view. It
does not expose an extensible runner lifecycle.

### Closed state execution contract

The conceptual author-facing contract is:

```text
trait State {
    type Config;
    type Context;
    type Input;
    type Output;
    type Facts;
    type Failure;

    fn execution() -> StateExecution<Self>;
}

StateExecution::pure(apply)
StateExecution::read<Request, Response, Capability>(request, apply)
StateExecution::effect<Request, Evidence, Executor>(request, settle)
```

The concrete Rust API may express the three cases through sealed traits, but they remain one closed
sum. There are no optional lifecycle callbacks and no public custom runner kinds. A third party may
register another compiled state, value, request, response, fact, capability, or executor. It cannot
add another semantic phase or event protocol.

The callbacks are:

```text
pure:
  apply(StateFrame)
    -> Settlement

read:
  request(StateFrame)
    -> one immutable typed Request

  apply(StateFrame, CommittedObservation<Read, Response>)
    -> Settlement | InsufficientEvidence

effect:
  request(StateFrame)
    -> one immutable typed Request

  settle(
      original StateFrame,
      CommittedRequest<Ensure, Request>,
      CommittedObservation<Ensure, TerminalEvidence>
  )
    -> Settlement | InsufficientEvidence | InvalidEvidence
```

`Settlement` is exactly `Succeeded { output_bindings, fact_emissions }` or
`Failed { typed_failure_ref }`. A typed failure is domain truth and commits. Invalid inputs,
corrupt evidence, callback faults, noncanonical output, and broken contract invariants are
execution/integrity failures: they block or reject and never become an invented semantic failure.

Read and effect request authorship is pure and total over the certified `StateFrame`. A domain
condition that may fail before request authorship belongs in an upstream pure validation state
that produces a stronger typed value. `InsufficientEvidence` appends no semantic transition and
cannot authorize a different read request or effect request. It permits another audited read retry
for the same request or another keyed `ensure` call that may return stronger evidence.

The exact same callbacks and canonicalizers run in live execution and replay. There is no
replay-specific reducer, verifier, or adapter.

The closed signatures exclude unsupported lifecycle variants and sealed authority misuse in safe
code; they do not capability-confine arbitrary compiled Rust. Selected state and capability
implementations are trusted, reviewed platform code whose executable identities are fixed in the
run manifests. Purity, determinism, total request authorship, audited-only IO, and absence of
undeclared ambient inputs are qualification obligations exercised by conformance tests. Replay
rejects reproducibility mismatches but cannot prove the absence of an undeclared influence that
happens to reproduce the same bytes.

### Minimal phase algebra

The complete semantic node phase algebra is:

```text
Unstarted | AwaitingEffect | Terminal
```

`Ready` is derived from `Unstarted`, the certified graph, exact available bindings, and dependency
rules. `Succeeded`, `Failed`, and `Skipped` are terminal outcomes, not phases. Read authorization
and observation are audit records, not node phases. `Blocked` is operational and appends nothing.
The run phase is only `Open | Closed`; success or failure is derived from terminal transitions.

The state flows are:

```text
pure:
  Ready
    -> materialize StateFrame
    -> apply
    -> StateTransitionCommitted(PureSettled)

read:
  Ready
    -> materialize StateFrame
    -> author one Request
    -> zero or more retry audit pairs for that same Request
    -> accept exactly one committed observation
    -> apply
    -> StateTransitionCommitted(ReadSettled)

effect:
  Ready
    -> materialize StateFrame
    -> author one Request
    -> StateTransitionCommitted(EffectRequested)
    -> zero or more audited ensure calls
    -> accept one committed terminal observation
    -> settle
    -> StateTransitionCommitted(EffectSettled)

skip:
  certified blocking proof
    -> StateTransitionCommitted(DependencySkipped)
```

A pure crash before commit permits recomputation. A committed read observation survives reducer or
settlement-append failure. The effect key, request, and original input manifest survive every
worker, and an awaiting effect can never author another request.

### Failure, skip, and forward-effect semantics

A committed `Failed` outcome is local to its node unless the certified graph says otherwise. It
produces no output or fact. A node whose required input can no longer be produced becomes
skip-eligible under its exact certified dependency rule and commits `DependencySkipped` naming the
blocking terminal sources. An independent node whose typed inputs remain available remains ready.

The canonical expanded spec contains:

```text
BlockingSource {
    producer_node_id,
    producer_terminal_transition_ref,
    required_output_position,
}

DependencyContract {
    required_input_sources,
    unavailable_input_rule,
}

RunTerminalContract {
    required_success_nodes,
    public_output_binding,
}
```

The baseline `unavailable_input_rule` is closed: once every possible producer of a required input
is terminal without that output, the consumer must skip and name the complete blocking set.
Optional absence is represented by the typed input schema and an ordinary produced value, not by a
custom scheduler callback. The transition's `node_id` selects this exact contract from the
certified spec; there is no second skip-rule identity or executable policy.

Every `BlockingSource` must be a direct certified producer for that required output position. Its
referenced terminal transition must be `Failed` or `DependencySkipped` and must not have produced
the required output. A skipped producer already names its own direct blockers, so replay follows an
acyclic chain without copying the transitive closure into each consumer. `RunClosed`, derived run
success/failure, public status, and any run-wide summary are not representable as skip sources;
run outcome is derived only after every occurrence is terminal.

Certification rejects a graph for which any terminal combination of `Succeeded`, `Failed`, and
`Skipped` leaves readiness, skip legality, public output, or run result undefined. Once every
occurrence is terminal, the run is `succeeded` exactly when every `required_success_node` succeeded
and the certified public-output binding exists; otherwise it is `failed`. Alternative business
success conditions are authored as an ordinary typed join/decision state whose output becomes that
required binding, not as another runtime rule language. There is no runtime-authored multi-branch
truth table.

The baseline deliberately has no run-wide “first failure” fence. If effect A fails while an
independent effect B remains unstarted and ready, B may still commit `EffectRequested`. Keyed
convergence answers whether B can be redelivered safely; the certified graph answers whether B
should exist and remain ready.

An exact `PlanningProfile` may impose fail-stop semantics by injecting ordinary typed success gates
and dependencies over selected effects. That policy is visible in the expanded graph, transition
trace, and replay. A static gate may serialize effects more strongly than the former
concurrent-then-fence saga behavior. Preserving independent concurrency while dynamically refusing
only new effects after the first failure would require a separate certified scheduling policy and
is outside this baseline; runtime does not recover it through a hidden global branch.

### Authority-bearing execution types

Keep only the public or crate-visible phase types that prevent an unsafe crossing:

```text
StateFrame<S>
  verified typed view over one deterministic input-manifest candidate; no append authority

CommittedRequest<K, T>
  the exact read or effect request with durable authorization/transition authority

AuthorizedAccess<K, T>
  non-cloneable affine authority for one boundary operation

CommittedObservation<K, R>
  one verified observation that a state callback may examine

Settlement<S>
  typed success or typed domain failure ready for transition validation
```

`K` is a sealed `Read` or `Ensure` marker. The read request becomes committed in its authorization
batch; the effect request becomes committed in `EffectRequested`.
`AuthorizedReadAccess` and `AuthorizedEnsureAccess` may be aliases over the one implementation.
These authority types are publicly nameable so implementations in other crates can accept them,
but constructors and fields remain private. The pre-observation wrapper result, variant-specific
transition candidates, and erased catalog dispatch remain private.

### Computation ownership

All outcome-affecting computation belongs in the certified state contract:

- domain validation and canonicalization;
- read and effect request authorship;
- cross-run fact-query authorship and interpretation;
- exact observation acceptance and selection;
- safe-access-failure-to-typed-failure policy;
- read reduction;
- terminal effect-evidence verification;
- output and fact construction; and
- every computation replay must reproduce.

Operations, the exact entry-point planner, and the certified spec own only static topology, typed
pre/post injection and rewiring, exact typed bindings, dependency/skip rules, state execution
contracts, capability/executor identities, public-output bindings, and terminal conditions. The
planner is pure and sees no run data or live capability.

Runtime owns verified-view loading, deterministic readiness, exact input materialization, typed
callback dispatch, audited capability orchestration, canonical result validation, transition
construction, and exact-head retry. It owns no domain policy, protocol phase progression, business
retry count, compensation, finality policy, framework-origin branch, or public projection model.

A read capability owns one external application-protocol operation for the exact typed request,
including bounded protocol encoding/decoding, source validation, and safe error classification. It
cannot reduce a state, construct output/facts/failure, choose graph behavior, retry invisibly, or
define replay behavior. The architectural adapter role remains the private live-crate binding from
state-owned capability intent to the reusable transport. It owns neither a runtime lifecycle nor a
semantic execution layer.

The kernel runtime crate owns the sealed `AuthorizedAccess` and uncommitted/committed observation
types. A private live `CapabilityCatalog` binding accepts that authority and calls a lower
runtime-agnostic reusable transport. Runtime never depends on a domain live crate, and the affine
token does not infect a public transport API used outside MFM. The compile-time guarantee covers
invocation of a registered MFM capability entry, not independent use of the transport primitive.

The effect executor owns external convergence, delivery state, cross-effect resource coordination,
signing/delivery choices, and durable terminal evidence within its certified equivalence contract.
It cannot settle an MFM node.

Store owns append idempotency, exact-head compare-and-swap, atomic object admission, record and slot
legality, closure/tail rules, hashes, coordinates, and structural fold validation. It never
executes states or decides domain outcomes.

Cross-run fact selection is an ordinary read state using the reserved same-journal fact capability
defined below. The state authors the query from base inputs and interprets the committed response;
the capability owns only verified prefix scanning and deterministic response construction. A
completed `StateFrame` never authors the query used to construct itself. Public semantic output is
a certified binding. An ordinary pure state performs any real semantic projection; CLI/API JSON or
text rendering is transport presentation and cannot reinterpret the typed value.

### One runtime entry point

The only post-admission mutating execution API is:

```text
async fn drive_once(authority: RunAccessAuthority<Drive>) -> Result<DriveOutcome>

DriveOutcome =
    Advanced { journal_head }
  | Waiting {
        journal_head,
        reason: RetryableEvidenceGap | OperationalBlock | IntegrityBlock,
    }
  | Closed { closure_ref }
```

`drive_once`:

1. loads the certified spec, committed journal, and required objects into one `VerifiedRunView`;
2. purely derives one next action in certified node order;
3. performs at most one semantic transition or one audited application-protocol operation;
4. appends against the exact journal head; and
5. returns without retaining semantic process state.

One audited operation may append its authorization and observation as two distinct journal
commits. The “one action” bound is one new live operation, not one physical store append.

After committing an observation, `drive_once` purely scans it with the retained compatible
observations. It returns `Advanced` only when a settlement candidate now exists.
`Returned(Pending { .. })`
or all-`InsufficientEvidence` returns `Waiting::RetryableEvidenceGap` after preserving the audit
record; invalid evidence returns `Waiting::IntegrityBlock`. A later host call may retry according to
its backoff and budget, but `drive_until_waiting` stops and never hot-loops another live call.
Missing capabilities and other operational prerequisites return `Waiting::OperationalBlock`.

The crate-private action algebra is:

```text
CommitPure
CallRead
SettleRead
CommitEffectRequest
CallEnsure
SettleEffect
CommitDependencySkip
Closed
Blocked
```

`CallRead` and `CallEnsure` include authorization, at most one boundary operation, and mandatory
observation persistence for every surviving wrapper result. `SettleRead` considers matching
observations in journal order but lets the certified state callback accept or reject each one;
arrival time or worker policy never selects semantic evidence. `SettleEffect` invokes the exact
state terminal verifier.

Next-action derivation closes the insufficient-evidence case:

```text
scan compatible observations in journal order
  first Settlement     -> SettleRead | SettleEffect with that observation
  InvalidEvidence      -> Blocked integrity failure
  all insufficient:
    read               -> CallRead with the identical authored request
    effect             -> CallEnsure with the committed effect request
```

The scan is pure and may repeat. `SettleRead` and `SettleEffect` are selected only when they can
construct a settlement candidate, so `drive_once` cannot livelock on the same insufficient
observation. The next later `drive_once` after a retryable wait may derive `CallRead` or
`CallEnsure`; the automatic loop cannot do so in the same drive-until-waiting call.

On stale compare-and-swap, runtime reloads. Pure work may be recomputed. A committed observation is
reused and no live access is repeated merely because transition append lost the race.

The deterministic scheduler still emits dependency skips in certified order. Closure waits until
every occurrence is terminal. A missing deployment capability, corrupt evidence, or operational
prerequisite returns `Waiting`; it appends no semantic truth.

No worker attempt, execution claim, or lease is required for correctness. A host may repeatedly
call `drive_once` and choose wakeups, backoff, budgets, or leases, but those operational choices
cannot construct a request, select evidence, create a failure, authorize access, replace an effect,
or close the run.

### Minimal catalogs and replay

Process-private runtime bindings contain:

```text
StateCatalog
  (state_contract_ref, executable_identity_ref)
    -> erased typed request/apply/settle functions

CapabilityCatalog
  (
    capability_binding_ref,
    capability_operation_id,
  )
    -> pure schema/classifier verifiers plus optional live implementation
```

The immutable per-run state-implementation and capability-binding manifests select only the exact
catalog entries required by the certified spec. A process catalog may be a superset; unrelated
additions or removal of unselected entries cannot change the run contract. Admission and resume
fail closed when a selected entry is missing or mismatched. Catalog construction and binding
perform no semantic IO;
provider, source, chain, signer, or route probing belongs in an audited post-admission read state.
Injected and authored states enter this manifest identically; runtime dispatch does not retain an
origin bit or use a second catalog.
The reserved `mfm.journal.fact-selection.v1` entry is constructed internally from the exact
`RunJournalStore` used by the runtime; app assembly cannot override it or supply a separate fact
store.

Type erasure exists only at heterogeneous catalog dispatch and is checked against the selected
manifest, certified schemas, and executable identity. There is no separate runner identity, runner
factory, runner output event algebra, or adapter registry.

Replay loads the same selected `StateCatalog` entries and the verification-only portion of the same
selected `CapabilityCatalog` entries. It installs no live implementation or invocation authority,
walks the recorded transitions, and invokes the same pure functions. It does not run the live
scheduler or a replay broker.

## Planning-Time Typed Framework State Injection

MFM distinguishes three mechanisms that must not be conflated:

1. **Kernel structural invariants** are excluded by sealed types and certified construction inside
   the trusted API, then revalidated where persisted bytes, erased dispatch, or concurrency cross
   an authority boundary.
2. **Semantic safety policy** is ordinary typed state-machine behavior injected into the operation
   plan before certification.
3. **Operational telemetry** is derived observation that cannot affect semantic execution.

There is no framework pre/post lifecycle in runtime: no `FrameworkPrechecked`,
`FrameworkPostchecked`, `CommitReady`, generic hook, middleware callback, or plugin chain.
`drive_once` materializes one ordinary `StateFrame`, calls its closed pure/read/effect contract,
constructs one sealed transition candidate, and attempts one append. It never branches on whether a
node was authored or injected.

### Structural invariants are authority boundaries, not hooks

Inside the typed execution API, invalid authority combinations are unrepresentable:

| Invariant | Owning exclusion boundary |
| --- | --- |
| State, input, output, fact, request, evidence, and capability compatibility | Typed plan builders, sealed constructors, and certification |
| Exact committed inputs and source lineage | `VerifiedRunView` constructing `StateFrame<S>` |
| Read/effect authority and evidence kind | `CommittedRequest`, affine `AuthorizedAccess`, and `CommittedObservation` |
| Typed success or domain failure | Closed `Settlement<S>` constructors |
| Transition body and exact predecessor | Private `CommitTransition` construction from one verified view |
| Append uniqueness, hashes, atomicity, and closure legality | The store transaction |

The last verification at an erased catalog, replay decoder, object loader, or store boundary does
not introduce another phase or extension point. It is the predicate required to reconstruct the
sealed type from untrusted bytes or to win a concurrent append. A callback fault, wrong schema,
noncanonical encoding, corrupt evidence, value rejected by the closed persisted schema or reviewed
classifier, stale head, or illegal batch fails closed and never becomes a semantic transition.
Framework states cannot weaken or replace these boundaries.

### Deterministic plan expansion

Separately traceable framework semantics use deterministic typed plan expansion:

```text
operation builds authored typed program
  -> exact entry-point PlanningProfile
  -> bound pure planner expands the program exactly once
  -> certification independently reruns and verifies expansion
  -> CertifiedTypedSpec
  -> thin runtime executes only ordinary certified nodes
```

The operation builder is the authoring surface, but operation code does not choose whether a
mandatory framework policy applies. Every published entry point binds and documents one exact
content-addressed profile:

```text
PlanningProfile {
    profile_ref,
    planner_contract_ref,  # exact algorithm/executable identity, not an abstract interface
}
```

Composition, order, configuration, applicability, and stable injected-node identity rules belong
to that planner contract rather than a generic framework rule algebra. A policy change creates a
new profile or planner-contract identity. There is no “minimum profile,” mutable policy merge,
public `NodeOrigin`, or runtime framework catalog.

The certificate binds the entry-point contract, retained canonical authored-program reference,
exact profile, planner contract, and expanded-spec hash. Verification reruns the bound pure planner
over the retained authored program and requires byte-identical canonical expansion before minting
`CertifiedTypedSpec`. Planning occurs once after child-operation composition and before final node
identities freeze. Injected nodes are not recursively reinstrumented. Only the expanded certified
graph is runtime authority.

### Typed pre/post chains

A framework pre-state is an ordinary state that consumes and reproduces the protected state's exact
typed input. A framework post-state consumes the protected output and produces the effective
output:

```text
InputHandle<I>
  -> FrameworkPre<I>
  -> InputHandle<I> with framework producer lineage
  -> ProtectedState<I, O>
  -> raw OutputHandle<O>
  -> FrameworkPost<O>
  -> effective OutputHandle<O>
```

The pre-state can reuse the same content-addressed input bytes while producing new typed lineage.
The protected state's `Input` remains `I`; there is no persisted permit, callback wrapper,
framework-specific `StateFrame` field, or runtime readiness rule. Multiple policies form the one
deterministic input/output chain owned by the exact planner contract.

The planner rewires the protected input through every required pre-state and rewires every original
consumer, public-output binding, fact-candidate path, and ordinary cross-run export through the
final post-state. Branded planning handles make those bindings non-forgeable in the authoring API.
Certification independently rejects a missing pre-state, raw-output consumer/export, recursive
injection, or graph that differs from the exact planner result.

The expanded spec retains a certified mapping from each authored logical output identity to its
effective expanded output reference. Same-run wiring uses only the effective handle. Cross-run
admission resolves the logical identity through this map; raw protected transitions remain
available only for trace inspection and explicit evidence-only correction roles and cannot satisfy
an ordinary typed input, fact, public output, or approved-result slot.

A pre-state failure is an ordinary typed failure. The protected state and success-dependent
post-states then receive ordinary `DependencySkipped` transitions. A protected failure or skip
produces no successful output, so its post-states skip. A post-state failure preserves the raw
protected transition for traceability but produces no effective output.

An external pre-state records only what it observed. If safety can change between the check and a
mutation, the destination or effect executor must enforce the condition atomically; a prior
observation cannot prove a later external fact.

Fact emission requires the same precision. A fact emitted by the protected transition is
authoritative immediately; a later post-state cannot hide or retract it. A post-gated protected
state must therefore emit zero facts and produce typed `FactCandidates<F>`. The planner threads
those candidates through the complete post chain, and only the final effective post-state may emit
them. Certification rejects an incompatible direct fact emitter or intermediate emission. There
is no unpublished, promote, or retract fact lifecycle.

Framework states receive only ordinary certified typed inputs, config, context, and their normal
committed read observation where applicable. They cannot inspect arbitrary journal records,
projections, runtime internals, or raw provider material.

For a protected effect state, the enforceable order is:

```text
FrameworkPre settles
  -> typed input reaches the protected state
  -> EffectRequested commits
  -> audited ensure authorization and invocation
  -> committed observation
  -> effect settle verifies terminal evidence
  -> FrameworkPost becomes ready
```

A framework pre-state is not a substitute for an atomic external conditional write, and a
post-state cannot authorize, reinterpret, or retroactively make safe an already applied mutation.
Evidence acceptance required for `EffectSettled` remains in the protected effect state's pure
`settle` callback.

An ordinary downstream post-state runs only when its required typed input is produced. It is not a
generic `finally` hook after success, typed failure, and skip. Telemetry covering every terminal
outcome derives from journal records. If a semantic policy must run after every terminal outcome,
the operation must model an explicit typed outcome value and normal dependency path; runtime gains
no special finally protocol.

Injected nodes obey the same pure, read, and effect protocols, transition trace, audit, keyed
convergence, replay, and failure/skip rules as authored states. An exact published profile may
inject an effect only when that external action is deliberately semantic, visible in the expanded
graph, and qualified under the ordinary effect contract. Guaranteed compliance delivery may use
such an explicit effect; best-effort telemetry remains derived.

The exact profile can guarantee that protected input flows through its complete pre-chain, no
successful protected output becomes usable or publishable before its complete post-chain succeeds,
and every injected occurrence settles or is explicitly skipped before run closure. It does not
claim generic finally behavior or finite-time physical execution after permanent worker or
capability loss.

### Telemetry is not a semantic hook

Metrics, tracing spans, duration, CPU use, queue delay, logs, and unsuccessful pure recomputations
are operational observations. They are emitted best-effort from process-local driver spans or
derived asynchronously from committed journal records. They:

- cannot block, fail, schedule, settle, skip, or close a run;
- are excluded from state digests and transition inputs;
- cannot mint authority; and
- are not replayed as state computation.

Committed-transition telemetry comes from transition records; external-access telemetry comes from
authorization and observation records. Generic telemetry events are not added to the semantic
journal.

If a value must be durable, replayed, or allowed to change an outcome, it is not telemetry. It must
be an explicit typed input, output, fact, access observation, or structural journal field. If
delivery to an external compliance sink must itself be guaranteed, it is an explicit qualified
effect in a user-visible operation, not a framework hook.

## Run Closure

`RunClosed` is an explicit hash-bound journal record in the same batch as the terminal semantic
transition.

Its payload is non-circular:

```text
RunClosed {
    terminal_transition_record_hash,
}
```

The terminal transition record is hashed independently before the commit envelope. `RunClosed`
binds only that record hash, while the enclosing `commit_digest` binds both records and their
assigned coordinates. State digest, result, facts, and public output remain solely in the terminal
transition and are not duplicated as closure authority.

After append, the verified view derives:

```text
SemanticClosureCoordinate {
    terminal_transition_ref,           # run sequence, ordinal, and record hash
    containing_commit_digest,
}
```

The derived coordinate is not embedded in either record, so no digest refers to itself. The
containing commit digest is the fixed semantic head. A separate current journal head may advance
through legal audit-only tail commits.

Runtime and replay rederive terminality from the certified graph, transition results, pending
effects, facts, and public output and reject an early, missing, or mismatched closure.

`RunClosed` is illegal until every certified occurrence is `Terminal`. This uniformly includes
unstarted pure/read/effect nodes, effects still `AwaitingEffect`, and occurrences waiting for a
certified `DependencySkipped` transition. A failure on another branch cannot close a run around an
externally ambiguous or applied-but-unsettled effect. The final settlement or skip transition
carries `RunClosed` in its batch.

Closure prohibits:

- another semantic transition;
- a new external-access authorization;
- another effect request;
- public-output replacement; and
- correction of the closed history in place.

It permits only one audit observation for each unmatched pre-closure authorization. Those tail
records leave the semantic state digest and closure coordinate unchanged.

## Objects, Evidence, and Retention

Large typed values remain immutable content-addressed objects:

```text
ValueRef {
    artifact_id,
    content_digest,
    evidence_hash,
    schema_id,
    semantic_type_id,
    role,
    byte_length,
    media_type,
    producer_binding,
}
```

The journal record carries exact object references and evidence. An object may be staged before a
commit, but gains run authority only when atomically admitted and first referenced by a committed
record.

For an object produced by the same candidate, `producer_binding` is the relative
`ThisRecord(field_path)` form; transition output/fact slots use its typed ordinal form. The full
coordinate-bearing producer reference is derived after commit and is never embedded back into the
object's content digest or producing record.

The object store is the payload annex of the journal, not a second lifecycle store.

Initial retention is indefinite for:

- certified spec, certificate, canonical authored program, exact planning profile, planner
  contract, config, seeds, and contexts;
- transition input manifests;
- transition input and output values;
- external typed read and effect requests;
- committed representable access results;
- safe failure evidence;
- fact responses and descriptors;
- effect terminal evidence;
- public output; and
- transition and closure dependencies.

The refactor deletes projection-owned retention manifests and synthetic retention nodes. Future
garbage collection requires a separate design proving complete dependency closure across run and
cross-run references.

## Facts and Cross-Run Inputs

Facts are outputs of their producing transition. Direct same-run consumption uses ordinary
certified graph edges. Cross-run selection is an ordinary `StateExecution::read` using the reserved,
non-overridable `mfm.journal.fact-selection.v1` capability backed by the same journal store:

```text
typed base inputs
  -> state authors one FactSelectionRequest
  -> append ExternalAccessAuthorized
  -> audited same-journal fact-selection capability
  -> append ExternalAccessObserved(FactSelectionResponse)
  -> the same state reducer interprets the committed response
  -> StateTransitionCommitted(ReadSettled)
```

There is no pre-execution fact materializer, fact-query field in `StateFrame`, query runner, or
separate fact store.

The canonical request is bounded, ordered, float-free, and state-authored:

```text
FactSelectionRequest {
    version,
    producer_scope: OtherRunsInTenantScope,
    queries: [
        {
            fact_descriptor_ref,
            canonical_predicate,
            content_identity_filter?,
            ordering,
            limit,
            tie_break,
        }
    ],
}
```

`OtherRunsInTenantScope` is the only baseline producer scope. The runtime binds it to
`RunAdmitted.tenant_scope_id`; state code cannot name or override another tenant. The reserved
capability excludes the current run and rejects every fact whose producing admission has another
tenant scope.

Adaptive selection uses another typed read state whose output feeds the next request. Same-run
facts are excluded from this capability because their dataflow must be visible in the certified
graph.

The committed reviewed response is:

```text
FactSelectionResponse {
    request_digest,
    frontier: StoreReadFrontier {
        store_scope_id,
        store_epoch,
        store_commit_order,
    },
    results: [
        {
            query_ordinal,
            selected: [
                {
                    fact_ref,
                    producing_transition_ref,
                    descriptor_ref,
                    subject_ref,
                    response_ref,
                    content_identity,
                }
            ],
        }
    ],
}
```

The frontier is exactly the `store_commit_order` of the response's
`ExternalAccessAuthorized` commit. Every lower committed order is already visible, the
authorization emits no fact, and later concurrent facts cannot enter the result. Empty selections
are explicit.

Before returning, the reserved capability verifies every producing transition and object binding,
rederives descriptor, subject, response, and content identity, evaluates the exact query, and
applies deterministic ordering, limit, and tie-break. Selected subject/response values remain
content-addressed objects; the ordinary typed read-response materializer resolves their declared
references without inline duplication or a fact-specific state loader.

`FactHistoryScan` is only the capability/store's private implementation algorithm. It:

1. scans the authoritative tenant-scoped subset of the dense global prefix through the
   authorization frontier;
2. locates fact-bearing transition records through the validated `emits_facts` routing column;
3. rehydrates every candidate from the journal and object annex;
4. verifies its transition, descriptor, response, and routing evidence; and
5. produces the complete deterministic response.

The state owns query construction and response interpretation. The capability cannot construct
state output, facts, typed failure, graph behavior, or transition authority.

Replay reauthors the same request from the retained base-input manifest, verifies the authorization
and observation, and verifies the response against authoritative history through the recorded
frontier before invoking the same state reducer. This proves omissions as well as selected-item
validity when replay reads the authoritative store snapshot directly.

A dense global commit order is not by itself an authenticated portable completeness commitment.
Hashes inside supplied commits prove their internal content and linkage; without the authoritative
store snapshot they do not prove that the exporter supplied the one authoritative commit at every
order.

The baseline supports fact-selection completeness only for a trusted same-store verifier scanning
the authoritative contiguous prefix. Every self-contained portable bundle reports selection
completeness as `Unverified`; it may verify the integrity and provenance of included facts, but it
cannot prove absence or that no qualifying fact was omitted. A portable global-prefix dump does not
upgrade that claim and must not disclose unrelated tenant records merely to imitate density.

A later RFC may add portable completeness only by choosing one concrete scope-safe census or
frontier-commitment scheme, exact trust root and revocation model, canonical proof schema, and
verifier. This RFC defines no portable-completeness proof type, verifier hook, or schema.

The baseline intentionally scans all fact-bearing transitions through the frontier rather than
claiming an index can select relevant embedded facts without complete routing. It does not rebuild
a universal run projection for every admitted run.

If measurement later requires a candidate index, it must:

- bind each row to a producing transition and fact identity;
- carry extractor/canonicalizer version;
- carry `indexed_through: StoreReadFrontier`;
- be used only when complete through the requested frontier;
- rehydrate and verify every candidate from journal authority;
- fail closed or scan the verified tail when incomplete; and
- be disposable and rebuildable without changing semantic truth.

Candidate verification catches false positives. Only a completeness frontier prevents silent false
negatives. An incomplete index scans the verified tail or fails closed; it never becomes semantic
authority.

## One Journal, One Fold, One Verified View

The public universal `ProjectionSnapshot` and its parts representation are deleted.

The read path becomes:

```text
CommittedRunJournal
  - native whole commits stored once
  - exact current journal head
  - derived latest semantic head
  - fixed semantic closure coordinate, when closed
  - canonical record iterator
  - required object references
  - compact private structural fold

VerifiedRunView
  - one CommittedRunJournal
  - one verified object set
  - one certified typed spec
  - semantic verification without copied lifecycle maps
```

Do not expose `CommittedRunStream -> RunJournalFold -> VerifiedRunView` as three public authority
layers. The fold is a private implementation detail of the committed journal or verified view.

`VerifiedRunView` never performs IO or “optionally obtains” evidence. It verifies evidence already
in the journal and exposes the exact legal next action. Only `drive_once` may decide that a read or
pending effect lacks sufficient committed evidence and enter the mandatory audited access
protocol.

Purpose-specific readers operate over that same authority:

- `TransitionTraceReader` returns exact before/input/result/output/after frames.
- The reserved fact-selection capability privately scans and verifies fact emissions, exposing
  only its typed response and verification result.
- `PendingEffectReader` returns exact unresolved request transitions for redrive.
- `RunObservationReader` scans commit envelopes for status/watch of one authorized run.
- replay invokes the same pure request/evidence/verifier contracts used live.

These are algorithms or opaque proof objects. They never persist a competing lifecycle snapshot.

A fold checkpoint may be introduced only after measurement. It must bind an exact journal-head
digest and fold-version identity and be discarded on any mismatch.

## Replay Contract

Replay performs zero live semantic-capability, provider, executor, network, filesystem-domain, or
signer IO. It may read only the explicitly supplied journal, object annex—including the complete
certification proof closure—and cross-run source dependency bundles through replay storage readers.
For a fact-selection transition, verified replay additionally requires an authoritative same-store,
tenant-scoped journal snapshot through the recorded frontier. A self-contained portable replay that
encounters such a transition cannot produce a verified fact-selection-completeness result: it
reports completeness as unverified. It may inspect the integrity and provenance of included facts
only through a separate non-authoritative inclusion view.

Before replaying transitions, it revalidates `RunAdmitted` and every `CrossRunSourceRef` through the
same predicate used by live admission. For each source, the portable dependency bundle contains the
source admission, certified spec/certificate, exact planning profile/planner identity and
authored/expanded graph proof, journal chain through the relevant effective or raw transition and
source closure, referenced output/evidence objects, and certified destination role. Replay
re-resolves effective post output or evidence-only legality; a missing or downgraded source proof
rejects verified replay rather than trusting the destination root hash.

For each semantic transition it:

1. verifies the predecessor head and before-state digest;
2. reconstructs every exact input binding from certified spec and retained objects;
3. reauthors the exact typed read or effect request where the state contract requires it;
4. verifies the exact consumed access observation and its result object;
5. invokes the same pure state verifier/reducer used live;
6. recomputes outputs, facts, binding delta, and after-state digest; and
7. verifies terminal closure when present.

Variant rules are closed:

- `PureSettled` reruns the pure reducer from its manifest.
- `ReadSettled` reauthors the one request, verifies the exact consumed observation against that
  request and manifest, and runs the same pure acceptance/reduction callback used live. A reserved
  fact-selection response additionally verifies the authoritative same-store snapshot through its
  authorization-order frontier. Portable replay cannot verify this variant in the baseline; its
  separate inclusion view may verify included facts without claiming selection completeness.
- `EffectRequested` reauthors the semantic request and kernel effect key without live mutation.
- `EffectSettled` reuses the request transition's exact manifest, verifies the terminal evidence
  envelope under its stated proof basis, and runs the pure settlement verifier.
- `DependencySkipped` verifies every unavailable source against the node's certified dependency
  contract without invoking the skipped state.

For access audit it:

1. verifies every authorization against its semantic anchor, complete capability binding,
   `capability_operation_id`, and typed request;
2. verifies at most one observation per authorization;
3. permits unmatched and indeterminate authorizations as honest audit history;
4. verifies safe-failure and result schemas;
5. for every representable executor `Pending` or `Terminal` observation—including audit-only,
   stale, and post-closure observations—verifies the complete newly retained delivery-audit suffix,
   predecessor chain, executor binding, effect key, request digest, proof, finite bounds, and
   one-prefix-chain legality, then rederives the private greatest frontier without executor IO;
6. permits a semantic transition to consume only explicitly referenced committed observations;
7. ignores audit records when deriving state unless a semantic transition references them; and
8. accepts post-closure observations only for pre-closure unmatched authorizations.

Replay proves that the recorded typed result is reproducible from certified inputs and accepted
evidence under the admitted compiled implementation. It detects a mismatch but cannot prove that
trusted compiled code never consulted undeclared ambient authority when the result happens to
match. It does not prove the exact number of physical remote calls or current external truth.

Replay independently verifies the exact planning profile, planner identity, and canonical plan
expansion. Injected states replay as ordinary states through the same `StateCatalog`; there is no
framework replay path. Replay emits no operational telemetry and constructs no runtime hook or
live capability registry.

## Explicit Corrective Runs

Generic saga policy, remediation roles, obligations, reverse ordering, manual terminalization, and
public `compensated` run modes are removed.

A corrective run consumes the root `CrossRunSourceRef` contract. `EffectiveOutputSource` resolves
the logical source through its certified plan expansion; for an unwrapped source, the source
transition is already effective. `EvidenceOnlySource` is admitted only into an explicitly declared
correction-evidence input slot. It cannot satisfy an approved domain value, ordinary output, fact,
public output, or equivalence claim merely because the raw transition succeeded.

Initially the source run must be semantically closed. Certification and admission verify the source
spec, closure, lineage, role, and effective-output resolution. The new run has its own certified
spec, invocation identity, exact inputs, effect keys, external-access audit, and output.

Correction uniqueness is a domain admission contract, not a second store or runtime path. Every
published correction entry point that promises at most one correction for a logical purpose uses
one stable `entry_point_operation_id` for that correction family. Its
`operation_contract_ref` defines and verifies the canonical domain preimage:

```text
source_bindings                  # exact canonical source positions and verified refs
typed_correction_purpose
affected_resource_or_subject
explicit_sequence?               # present only for legitimate installments/supersession

invocation_identity = H(
    "mfm.correction-invocation.v1",
    canonical(
        source_bindings,
        typed_correction_purpose,
        affected_resource_or_subject,
        explicit_sequence?,
    ),
)
```

The domain entry point derives the ordinary `invocation_identity` and rejects a caller-supplied
substitute. The store needs no correction-specific key or verifier: its existing uniqueness of
`(store_scope_id, tenant_scope_id, entry_point_operation_id, invocation_identity)` attaches
concurrent callers only when the exact root candidate is unchanged and rejects changed root
material. Contract or implementation version changes never mint another correction identity; a
changed versioned root conflicts instead of attaching or creating a duplicate. A legitimate
versioned supersession must be represented by an explicit typed purpose or sequence under the
domain contract. A domain that legitimately permits several corrections represents their
distinction through the typed purpose, affected resource, or explicit sequence rather than through
an arbitrary fresh identity. Two differently named entry points do not share an at-most-once
guarantee; they must consolidate on one stable `entry_point_operation_id` or rely on a stronger
domain/executor atomic precondition.

There is no hidden liveness guarantee between source closure and corrective-run admission. If
guaranteed initiation is required, a domain recovery controller or outbox scans source transitions
and admits corrections through that same derived invocation key.

A domain may report `compensated` only when typed evidence proves the claimed equivalence under
explicit precondition, touched-resource, and concurrency assumptions. Completing a corrective
effect without that proof may report only `remediation_completed`.

## EVM Qualification

The current EVM transaction implementation does not qualify for keyed `ensure`.

It has useful reusable primitives:

- typed EIP-1559 intent and transaction construction;
- deterministic signing-provider contracts;
- expected transaction-hash derivation;
- single-exchange transport with exact hash checking;
- transaction lookup, receipt, and finality evidence; and
- pure domain verification.

It currently stores signed bearer bytes only in a process-local cache and performs lookup-only
recovery after uncertainty or restart. MFM-local sender lanes do not reserve nonces against another
wallet, process, or deployment.

The target is a durable wallet or relayer that atomically owns:

```text
effect key
  -> semantic request digest
  -> exclusively coordinated sender and nonce
  -> fixed semantic transaction payload
  -> append-only replacement-candidate lineage
  -> signing or secure reconstruction
  -> delivery and terminal evidence
```

It owns the signer account exclusively or participates in one coordinator spanning every actor
that can use that account. It also owns nonce allocation, signing, rebroadcast, replacement policy,
and durable observation. Every replacement keeps the same sender, nonce, destination, value, call
semantics, and certified policy; only explicitly approved fee/envelope fields may vary.
MFM audits each call it makes to this executor. Executor-internal JSON-RPC calls are outside MFM's
direct authorization/observation records but must appear in the executor's linked reviewed
delivery-attempt evidence.

MFM verifies that terminal transaction, receipt, and finality evidence satisfies the committed
semantic request. Evidence includes a non-bearer decoded transaction commitment covering chain
identity, sender, nonce, destination, value, calldata semantics, executed candidate hash, receipt,
block, and assurance frontier, plus the declared proof/attestation basis for request-to-envelope
correspondence. Raw signed transaction bytes remain absent from MFM journal, artifacts, public
status, errors, and replay.

`SubmitEvmTransactionState` remains unregistered until the executor passes:

- same-key/different-request rejection;
- concurrent and delayed ensure;
- exclusive signer-account coordination across processes, deployments, wallets, and other actors;
- restart and disaster recovery;
- response loss and already-known transaction behavior;
- success and revert evidence;
- finality and pre-resolution reorganization;
- replacement-equivalence policy;
- terminal evidence retention;
- duplicate delivery cost review; and
- no-secret/no-bearer persistence tests.

EVM initially omits a generic not-applied terminal outcome. Lookup absence, timeout, mempool
absence, and nonce observation do not prove permanent non-executability.

### Durable reference-executor gate

The generic executor, binding, evidence, restore, and failover schemas are not frozen solely from
mocks. Before the complete journal schema freezes, one narrow durable reference executor must
implement the contract against a convergence-safe semantic destination. A transactional durable
queue whose enqueue key is the destination identity is sufficient; it need not enable EVM.

The prototype must exercise:

- same key and same request under repeated, concurrent, delayed, and post-terminal `ensure`;
- same key with a different request or binding;
- executor crash before and after destination mutation;
- terminal tombstone retention;
- append-only evidence strengthening;
- restore, migration, stale replica, rollback, and split-brain attempts;
- exact original-binding routing across an implementation upgrade; and
- positively acknowledged delivery authorization before target entry, unmatched intent, linked
  outcome, prefix-frontier proof, attempt/frontier-bound exhaustion, and no credentials or bearer
  material.

Its binding and evidence objects become golden-vector inputs. If the prototype requires a field or
authority not expressible by the generic schemas, those schemas change before the memory,
PostgreSQL, runtime, and replay implementations proceed independently. EVM qualification remains a
later domain gate over the already proven generic contract.

## Security and Redaction

Journal and object surfaces must never contain or derive identifiers from:

- credentials, authorization headers, cookies, or bearer tokens;
- URLs, hostnames, user information, query strings, redirects, proxy details, DNS/TLS diagnostics,
  sockets, or filesystem paths;
- private keys, mnemonics, decrypted key material, or signer sessions;
- raw signed transactions or other submit-ready bearer payloads;
- raw provider bodies not admitted by a reviewed public schema;
- raw error strings, debug representations, source chains, or backtraces; or
- hashes or fingerprints used as surrogates for credentials, bearer values, secret endpoints,
  unrepresentable provider bodies, or other prohibited data.

A reviewed public protocol identity is not a prohibited surrogate merely because the protocol
derives it from a payload that MFM must not persist. For example, an Ethereum transaction hash may
be retained as the public identity needed to verify delivery and finality even though it is derived
from raw signed transaction bytes. This exception never admits the source bytes. Capability review
must establish that the identifier is bounded, public, non-secret, non-bearer, and necessary for
correctness.

Credentials are injected below the audited boundary after authorization commits. The authorization
contains only the reviewed public semantic request and a reference to the admitted capability
binding, which may contain an optional closed source scope.
Provider-generated request IDs are omitted unless a capability-specific review proves them bounded,
non-secret, non-bearer, and necessary.

`safe_failure` contains only:

```text
stable_code
failure_class
boundary_stage
optional reviewed coarse_size_class
optional reviewed redacted diagnostic_ref
```

These are closed capability-specific values. The audit API accepts no arbitrary string, metadata
map, provider error object, request body, response body, `Display`/`Debug` output, or error source
chain. If a failure cannot be classified safely, it records only `UnclassifiedFailure`.

The no-secret contract is enforced by closed persisted schemas, private constructors, bounded
classifiers/canonicalizers, implementation review, and adversarial tests. Rust types exclude
unreviewed representation paths after construction; they cannot decide whether arbitrary
semantically meaningful bytes are secret. Selected compiled implementations therefore remain
trusted qualification subjects rather than sources of an absolute type-level secrecy proof.

Operational timestamps may live in commit envelopes but are excluded from semantic hashes unless a
separate reviewed contract requires them. Worker identity, hostname, PID, lease token, and process
attempt count are not semantic fields.

Audit order, frequency, source scope, and wall-clock time remain privacy-sensitive even when they
are not secrets. Normal run status and transition output do not expose audit internals. A
privileged versioned audit export defaults to omitting source scope and precise time and remains
subject to the unresolved retention policy in `Material Uncertainties`.

Hashed structures remain canonical, domain-separated, and float-free. Bounded-input validation
applies before hashing or retaining provider-controlled content.

## Canonical Schema and Golden-Vector Gate

The logical schemas in this RFC become implementation authority only through one versioned
canonical schema annex. Rust layout, serde defaults, database column order, and backend-specific
representation are never hash contracts.

Before memory and PostgreSQL implement the cutover independently, the annex freezes:

- every top-level record and transition-body variant;
- every logical key, including admission, node/slot, observation, and closure uniqueness;
- `run_id`, canonical `node_id`, transition/output/fact references, and `effect_key`;
- genesis, predecessor, semantic-state, candidate, record, and commit digest preimages;
- capability and executor binding objects;
- input manifests, binding deltas, facts, outputs, evidence, and cross-run sources;
- object-path bindings and artifact-admission intents;
- enum tags, field names, integer widths, ordering, optional absence, explicit null where legal,
  empty strings, empty collections, and union discriminants; and
- schema/version rejection behavior.

The vector corpus contains canonical bytes plus every derived digest/reference for genesis and each
legal batch and transition variant. It includes negative vectors for reordered maps, duplicate
paths, omitted required fields, unknown tags, absent-versus-empty substitutions, floats,
noncanonical numbers, cross-binding substitution, and same-key/different-content conflicts.
Memory, PostgreSQL, runtime, replay, trace export, and the durable reference executor consume the
same corpus and shared domain-free codec/digest/fold implementation.

Schema freeze follows the production-read and durable reference-executor prototypes. Those
prototypes may change the candidate schemas; after freeze they cannot silently add a field or
reinterpret a tag. A deliberate persisted-contract change requires a new version and, under this
pre-production cutover, explicit rejection rather than a compatibility reader.

## Store and Postgres Shape

A concrete initial schema can remain normalized:

```text
store_identity
  store_scope_id
  store_epoch

store_schema_metadata
  schema_contract_version

store_commit_order
  current_order

journal_commits
  run_id
  run_sequence
  append_request_id
  candidate_digest
  predecessor_kind
  predecessor_run_sequence?
  predecessor_commit_digest?
  commit_digest
  store_commit_order
  record_count
  committed_at

journal_records
  run_id
  run_sequence
  store_commit_order
  ordinal
  record_id
  record_schema_id
  spec_hash
  logical_key
  record_hash
  canonical_payload
  emits_facts

artifact_blobs
artifact_admissions
commit_artifact_bindings

configured_values                         # pre-admission current configuration
```

Ordinary B-tree indexes over authoritative envelope columns are storage implementation details, not
semantic models.

`emits_facts` and the copied `store_commit_order` are store-derived routing fields, not
caller-owned truth. `emits_facts` is payload-derived and covered by `record_hash`.
`store_commit_order` is assigned later, covered by the commit envelope and `commit_digest`, and
must equal its containing commit. PostgreSQL uses a partial index equivalent to:

```text
(store_commit_order, run_id, run_sequence, ordinal)
WHERE emits_facts
```

The initial fact scan's completeness trusts PostgreSQL table/index integrity just as journal-head
lookup trusts the commit table. A query cannot discover a corrupted false routing value in a row it
does not read. If the threat model requires per-query detection of arbitrary database/index
corruption, it must scan every transition rather than use routing.

Required database constraints include:

- primary key `(run_id, run_sequence)` for commits;
- unique `(run_id, append_request_id)`, `commit_digest`, and `store_commit_order`;
- primary key `(run_id, run_sequence, ordinal)` and unique `record_id` for records;
- foreign keys from records and object bindings to their commit, admission, and content-addressed
  blob;
- a store-wide unique admission logical key for
  `(tenant_scope_id, entry_point_operation_id, invocation_identity)`, plus closed
  logical keys for one run admission, one closure, one transition identity, and one observation per
  authorization; and
- conflict checks for idempotent object admission and every hash/digest identity.

`store_identity` is a singleton immutable after bootstrap. Scope and epoch rotation is forbidden
once any run is admitted. A destructive reset must generate both a never-before-used
`store_scope_id` and a fresh `store_epoch`; reusing the old scope with only a new epoch is invalid.
Only verified non-rollback backup restoration or failover continuation of the same authoritative
lineage preserves both values. `store_schema_metadata` may evolve only through reviewed migrations
and never changes the identity bound into roots, facts, or effect keys.

One `(store_scope_id, store_epoch)` has exactly one authoritative writable journal lineage. A
database/HA generation fence outside the semantic runtime must cover every admission, transition,
authorization, observation, and closure append—not only effects. Before restore or failover
promotion, it permanently fences every old/sibling writer and proves that the candidate contains
the unique latest published commit prefix. A restored clone cannot serve writes concurrently under
the same identity. If a published suffix may be missing, the store fails closed; divergent
histories are never merged or resumed as one store.

The HA/WAL consensus or primary-fencing mechanism is irreducible storage authority, not a second
run-lifecycle model. Worker leases, process epochs, and executor fences cannot substitute for it.

The application role has insert/select access but no update, delete, or truncate access to
`store_identity`, journal commits, records, blobs, admissions, or bindings. Owner-only guard
triggers reject those operations even if application SQL regresses; migrations use a separate
owner path.

`store_commit_order` is a protected singleton, not a generally writable counter. The application
role has no direct insert, delete, truncate, or arbitrary update privilege. A sealed store
procedure, deferred constraint trigger, or equivalent permits only `current_order + 1`, atomically
coupled to exactly one inserted commit carrying that order. Store open/load verifies:

```text
current_order
    == max(journal_commits.store_commit_order)
    == count(journal_commits)           # committed orders are dense from one
```

An empty store uses zero. Any mismatch is corruption and fails closed.

Postgres append ordering is:

1. stage object bytes without granting authority and finish content verification;
2. begin one `READ COMMITTED` transaction and acquire the run lock;
3. resolve append idempotency, load the exact current head, and validate the sealed candidate,
   objects, logical identities, closure/tail rules, and whole-batch fold;
4. acquire the global store-order row lock last;
5. assign order-dependent coordinates, derive record IDs, materialize the commit envelope and
   `commit_digest`, insert every commit/record/admission/binding row, and advance the allocator; and
6. commit.

No lower store order can become visible after a higher frontier because assignment and publication
share the locked transaction. Committed orders are dense from one: the allocator increments by one
only in the transaction that inserts that commit, and rollback rolls the increment back. Overflow
fails before publication. This density lets an authoritative same-store scan prove that it covered
the complete prefix through its frontier; it does not create portable authority.

A failed or connection-ambiguous `COMMIT` returns `OutcomeUnknown`, never `NewlyAppended`; callers
resolve it through `append_request_id`. This is mandatory for an authorization append because only
a directly observed positive result can mint live authority.

Do not add persisted run-status, cell-state, effect-phase, saga, attempt, retention, or closure
snapshot tables. Do not add a generic MFM outbox table: the pending effect transition is the
outbox.

Mutable operational coordination is optional:

- A worker lease may reduce duplicate work but never grants semantic authority.
- Client observation cursor state may remain when the API promises durable opaque cursors.
- Wake hints and queues may select work but must be reconstructible from journal state.

The memory backend stages one complete candidate state and performs one infallible swap only after
all digest, overflow, identity, object, routing, idempotency, closure, and fold checks succeed. It
must not advance a counter, mutate object authority, or publish an idempotency lookup before a
later fallible step. PostgreSQL and memory share the same canonical digest and fold implementation
and must have injected-failure parity for every legal batch.

## App, CLI, and REST

Public run status reduces to:

```text
active
succeeded
failed
```

`active` is exactly `run_phase = Open`. `succeeded` and `failed` both require
`run_phase = Closed` and derive from the certified terminal/public-output contract and terminal
outcomes. No separately persisted status may disagree with that fold.

An active run may expose:

- current runnable node;
- pending effect key and safe executor status;
- redacted blocked reason; and
- latest journal and semantic heads.

Complete transition data is not public merely because its closed schemas contain no secrets. The
`TransitionTraceReader`, external-access audit reader, portable export, and their
content-addressed object dereferences are privileged surfaces. There is no standalone arbitrary
object-reader API and no generic object grant. Each dereference inherits the enclosing
`ReadPublic`, `Replay`, `InspectTrace`, `InspectAudit`, or `Export` authority and may expose only
the objects legal for that surface. Before loading records or objects, the reader validates that
authority for the exact tenant, store, run, and purpose and verifies that every requested object is
reachable from that run's admitted journal/object closure. `ReadPublic` reaches only the certified
public-output closure; the privileged grants reach only their reviewed trace, audit, replay, or
export closure. A record reference, artifact digest, fact identity, run id, or export manifest is
not bearer authority.

The ordinary CLI/REST/API surface requires `ReadPublic` and defaults to status plus the certified
public output. It does not dereference transition inputs, non-public outputs, facts, observations,
or evidence. A separately authorized trace inspection may expose:

- transition identity and containing commit;
- before/after state digests, `Open | Closed` run phase, exact node phase, and terminal outcome;
- exact named input lineage;
- typed request identity;
- consumed access observation;
- result, outputs, facts, and evidence; and
- closure identity when terminal.

External-access audit exposes safe statuses:

```text
authorized_unobserved          # CrashAmbiguous
returned
did_not_enter
indeterminate
```

The semantic run export is fixed at its semantic head or closure coordinate. A privileged semantic
or audit export is scoped to the authorized tenant/run and records that scope in its export
manifest, but the manifest grants no later access. Dereferencing a cross-run source from an export
or trace requires separate authority for the exact source run; without it, the reader exposes only
the reviewed redacted source identity. The audit export is explicitly “complete as of journal head
H” and, for each executor effect, “F is the greatest executor frontier committed to this MFM
journal at H.” It never presents F as the executor's current history. It may claim executor-complete
delivery history only when the bound verifier proves a sealed terminal frontier that cannot gain
another target-attempt record. An unmatched MFM authorization, asynchronous work after
`Returned(Pending)`, or later evidence strengthening can all make a newer frontier arrive.

It may expose stable public capability, operation, request, result, and failure identities. It must
not expose credentials, bearer bytes, raw provider errors, secret-bearing routes, or executor
vault references.

There is no generic manual “mark successful,” “mark not applied,” “abandon effect,” or
evidence-submission endpoint.

The app exposes `drive_once` directly or implements `drive_until_waiting` as a mechanical loop over
it. CLI/REST may choose when to call that service and render `DriveOutcome`; they cannot select the
next node, construct state requests, choose observations, install execution hooks, or interpret
typed results.

## Complete Cutover and Deletion Scope

### Program, spec, and certification

Delete:

- side-effect submit/verify node pairs;
- generic saga and remediation roles/pairs/policies;
- manual-resolution policy and evidence roles;
- generic receipt/finality phase policy;
- resource-claim classes;
- extensible runner-kind and adapter-binding semantics;
- static fact-query materializer entries and direct cross-run fact input bindings;
- synthetic completion and retention nodes; and
- `mfm-manual-auth`.

Add:

- complete transition input/output/evidence contracts;
- one closed `StateExecution` contract with pure, read, and effect cases;
- `StateFrame`, exact typed failure, and evidence-verdict contracts;
- external-read typed request and response contracts;
- `FactSelectionRequest`, `FactSelectionResponse`, and the reserved same-journal capability
  contract;
- audited read-capability and executor-binding identity;
- pure effect-request authorship;
- keyed executor identity and convergence assurance;
- explicit executor/domain cross-effect ownership;
- pure terminal evidence verification;
- deterministic framework plan expansion, exact planning-profile/planner identity, retained
  authored-program proof, and effective-output contracts; and
- spec-resolved effective-output and correction-evidence cross-run source roles.

### Events and store

Delete:

- state worker attempt start/completed/interrupted/failed events;
- standalone cell produced/skipped lifecycle events;
- standalone fact lifecycle events as semantic authority;
- unpublished, approved, promoted, or retracted fact variants;
- side-effect intent, claim, takeover, preparation, start, submission, receipt, confirmation,
  ambiguity, and failure event families;
- resource-lane claim/release events and waiter protocols;
- generic saga, manual-resolution, retention, and old completion events;
- public `ProjectionSnapshot` and projection parts;
- side-effect ledger typestates and copied owned variants;
- saga terminal proof and retryable open-attempt algebra;
- `active_effect_keys`, resource-key routing, and journal locking for external resources; and
- projection-owned retention manifests.

Add:

- the five-record journal algebra;
- complete transition structural validation;
- external-access authorization/observation legality;
- newly-appended-only affine access authority;
- post-closure audit-tail validation;
- predecessor-linked commit digests;
- one private fold and one opaque verified view; and
- direct transition trace reads.

### Runtime and replay

Delete:

- worker attempt lifecycle and recovery;
- side-effect phase drivers and recovery classifiers;
- first/repeat effect recovery modes and permits;
- saga scheduling, remediation ordering, and manual-block branches;
- `RunnerKind`, effect-runner traits, custom adapter-binding specs, and custom erased-runner
  registration;
- erased node-runner contexts, invocations, outputs, event payloads, output settlements, and
  runner-kit builders;
- external-read runner/executor wrappers and adapter-owned state execution;
- replay-specific reducers, side-effect verifiers, semantic brokers, and phase maps;
- pre-execution fact selection, query-materializer registries, fact-specific state loaders,
  portfolio selection runners, and separate fact-query store/projection authority;
- framework completion and retention runners; and
- copied runtime history/view maps.

Add:

- one `drive_once` mutating entry point and one closed private next-action decision;
- the minimal `Unstarted | AwaitingEffect | Terminal` phase algebra;
- one process-private `StateCatalog` and `CapabilityCatalog`, plus exact immutable per-run selected
  state-implementation and capability-binding manifests;
- complete transition-frame construction and verification;
- audited capability wrappers;
- common bound affine access and committed-observation authorities;
- one read request and one accepted observation per read occurrence;
- the non-overridable same-journal fact-selection binding and its pure recorded-response verifier;
- generic typed read-response materialization of content-addressed value references;
- committed-observation-only reduction;
- keyed `ensure`;
- the exact same state callbacks for live and replay;
- transition/audit trace readers.

### App and binaries

Delete generic saga, manual-resolution, worker-attempt, resource-lane, side-effect-phase,
runner-factory, and adapter-lifecycle DTOs, commands, modes, and routes. App assembly supplies the
compiled `StateCatalog` and `CapabilityCatalog` but owns no execution callbacks or framework-hook
registry and cannot replace the reserved fact-selection binding.

Add transition inspection, safe access-audit inspection, pending-effect status, and the one
`drive_once` action plus an optional mechanical drive-until-waiting convenience. Gate admission,
drive, replay, public read, trace, audit, object, and export services with the exact transient
`RunAccessAuthority`; do not persist app principals or ACL state in the run journal.

### Postgres

Reset the pre-production schema baseline and reject old histories.

Keep authoritative commits, records, object blobs/bindings, store metadata/order, current
configuration, and only explicitly required operational cursor state.

Delete lifecycle mirrors, admission lanes/waiters, physical retention state, and redundant
per-event or per-projection artifact-evidence mirrors after their fields move into the one
hash-bound `commit_artifact_bindings` relation. Keep content-addressed blobs, artifact admission
evidence, and the canonical commit-to-object binding; they are required to prove both existing
inputs and newly admitted outputs.

Replace the physical fact projection with the reserved capability's private targeted history scan
initially. If measurements require a candidate index, add it only with a complete watermark and
parity tests.

### Documentation

Rewrite:

- `docs/design.md`;
- `docs/architecture.md`;
- persisted/public surface documentation;
- store, runtime, replay, app, CLI, and REST READMEs; and
- EVM transaction and Bitcoin RPC routing documentation.

Delete `docs/saga.md` after its honest guarantee losses are incorporated into the new design
contract. Explicitly delete the `CertifiedFrameworkLifecycle`, framework public-output runner, and
erased runner plan/factory. Rewrite the adapter taxonomy around its narrower private
capability-to-transport binding responsibility; delete only adapter-owned lifecycle, reducer, and
runner authority rather than the architectural role.

## Crash and Ambiguity Matrix

### Semantic transitions and journal appends

| Boundary | Required result |
| --- | --- |
| Admission acknowledgement lost | Recompute the same canonical `run_id`, resolve the original append identity, and attach to the exact existing root; never allocate another run. |
| Same invocation identity, different root candidate | Reject the admission conflict; do not fork the business invocation. |
| Before transition commit | No semantic state changed; pure computation may rerun. |
| Transition append acknowledgement lost | Reload by `append_request_id`, candidate digest, and expected head. Never infer absence from a transport error. |
| Objects staged but append fails | Objects are orphaned bytes without authority. |
| Two transitions race | Expected-head validation admits one. The loser reloads the winning state. |
| Restore/failover promotion | Fence every old/sibling writer and prove the unique latest published prefix before any append; fail closed on possible rollback. |
| Destructive store reset | Generate a never-before-used `store_scope_id` and a fresh `store_epoch`; never resume or recreate runs under the old scope. |
| Terminal transition commits | Its `RunClosed` companion commits atomically or neither commits. |
| Closure acknowledgement lost | Reload and return the fixed semantic result. |

### External-access audit

| Boundary | Required result |
| --- | --- |
| Before authorization commit | No affine access authority exists; the capability cannot be called. |
| Authorization append acknowledgement lost | Reload for audit, but mint no authority. A later call uses a new authorization. |
| Authorization commits, process dies before call | The record remains `CrashAmbiguous`; zero calls may have occurred. |
| Task aborts, panics, is cancelled, or is forcefully shut down after authority minting | If no sealed outcome survives, the authorization remains `CrashAmbiguous` even when the process has other surviving tasks. |
| Affine authority consumed, call in flight | Authorization proves the call was audited; outcome remains unknown. |
| Capability rejects before entry | Append `DidNotEnter`. |
| Capability returns a reviewed result | Append `Returned` before state reduction. |
| Capability returns malformed or unsafe data | Append `Indeterminate(unrepresentable_response)` with safe metadata only. |
| Call may have entered and returns an error/timeout | Append `Indeterminate`; do not infer external outcome. |
| Process dies after return but before observation commit | Authorization remains `CrashAmbiguous`; the return is not semantic authority. |
| Observation append acknowledgement lost | Reload by authorization reference and result digest. |
| Reducer fails after observation | Observation remains auditable; retry pure verification without another access when possible. |
| Stale call returns after semantic closure | Append one audit-only observation for its pre-closure authorization. |

### Keyed effect recovery

| Boundary | Required result |
| --- | --- |
| Before pending-effect transition | No effect authority exists; pure request authorship may rerun. |
| Pending-effect transition commits before any ensure call | Any worker may reload the same key/request and authorize `ensure`. |
| Two workers call `ensure` | Executor/native idempotency converges both to one key, digest, and logical operation. |
| Runtime-to-executor response lost | Authorize and call `ensure` again with the same committed request. |
| Keyed executor ledger entry commits but acknowledgement is lost | Executor reloads the key; it must not choose another operation. |
| Target mutated, executor crashes before terminal commit | The delivery authorization remains unmatched unless its linked observation committed before the crash; either case lacks terminal authority. Redelivery is legal only under the recorded downstream convergence proof; otherwise the executor remains pending and does not qualify for recoverable liveness. |
| Target response is lost | Executor observes or convergence-safely redelivers the bound operation; it never chooses another semantic operation. |
| Executor returns `Pending` | Record its mandatory delivery-audit reference in the access observation; semantic state remains pending. |
| Executor returns terminal evidence | Record it, verify purely, then attempt atomic settlement. |
| Settlement acknowledgement is lost | Reload the transition journal and return the settled result. |
| Delayed ensure arrives after settlement | Executor tombstone returns compatible evidence without another mutation. |
| Key/digest conflict or invalid evidence | Fail closed; keep the effect pending and surface a redacted operational incident. |
| Lease expiry, shutdown, or cancellation | Does not revoke a call, prove non-application, or authorize a different request. |

## Acceptance Tests

### Transition trace

- Persist one exact input manifest for every executed transition and one complete blocking-source
  proof for every dependency skip.
- Prove the input manifest is one content-addressed object: admitted by the first read authorization
  or its pure/effect-request transition and reused exactly thereafter.
- Verify seed, config, context, typed binding/output, fact, and cross-run lineage.
- Reject a cross-run `TransitionOutput` or `TransitionFact` in a later input manifest; admit
  cross-run values only through the root `CrossRunSourceManifest` and ordinary cross-run facts only
  through the reserved audited read.
- Reject a `CrossRunSourceRef` from another store scope, epoch, or tenant.
- Verify before-state digest against the predecessor fold.
- Verify output/fact/evidence binding and after-state digest.
- Verify `RunAdmitted` root lineage and canonical genesis/initial-state test vectors.
- Prove admission atomically binds the canonical authored program, exact planning profile/planner
  contract, every selected state-implementation and capability-binding manifest, cross-run source
  manifest, and complete source-proof object.
- Re-run the exact certificate-bound planner over the retained authored program and reject a
  profile, planner identity, or canonical expanded spec mismatch.
- Recompute every canonical `node_id` from its versioned identity preimage; reject duplicate,
  path-ambiguous, map-order-dependent, or differently expanded identities.
- Retry and attach the same logical admission across ambiguous `COMMIT`; reject changed root
  material and prove no second effect-key namespace appears.
- Reject every illegal transition-body field combination and verify the complete dependency-skip
  blocking set against the certified node contract.
- Reject a closure, derived run result/status, non-producer, or copied transitive blocker as a
  dependency-skip source; accept only direct failed/skipped producers missing the required slot.
- Enforce the certified node-class/body matrix and delay closure until every required
  `DependencySkipped` transition commits.
- Exhaustively exercise certified terminal combinations for representative fan-out/fan-in graphs;
  prove readiness, skip legality, public output, and success/failure are uniquely derived.
- Prove output and fact identities derive non-circularly from the committed transition and ordinal.
- Compare full-fold and incremental-fold results.
- Accept only the four exhaustive batch purposes and reject observation-plus-transition,
  authorization-plus-closure, reordered closure, and every multi-audit combination.
- Prove an effect-request batch binds both its input manifest and semantic request atomically.
- Prove one semantic transition per batch and whole-batch atomicity.
- Prove no full universal before/after snapshot is persisted.
- Export and rehydrate a transition trace without mutable current configuration.

### Typed execution runtime

- Compile and register states through exactly the pure, read, or effect `StateExecution` case;
  reject custom lifecycle/runner kinds and missing executable identity.
- Derive only `Unstarted`, `AwaitingEffect`, and `Terminal` node phases; prove ready and blocked are
  derived and worker activity creates no phase.
- Property-test that every registered read/effect request callback is deterministic and total over
  generated valid `StateFrame` values.
- Inventory selected planner/state/capability dependencies for ambient IO, time, environment, RNG,
  globals, and FFI; exercise conformance under varied process conditions and document that
  compiled-code purity remains a reviewed trust assumption.
- Distinguish typed `Settlement::Failed` from invalid inputs, callback faults, invalid evidence,
  and noncanonical output; prove the latter append no semantic failure.
- Prove one read occurrence authors one request, every retry reauthors identical content, and one
  settlement consumes exactly one state-accepted observation.
- Prove `InsufficientEvidence` appends no transition and cannot change a request.
- Prove all-insufficient read observations derive `CallRead`, all-insufficient terminal effect
  observations derive `CallEnsure`, and invalid evidence derives a blocking integrity result
  without a no-op settlement loop.
- Prove a newly committed pending/all-insufficient observation returns
  `Waiting::RetryableEvidenceGap`, makes `drive_until_waiting` stop after at most that one live call,
  and leaves retry timing/backoff to the next host invocation.
- Exercise every private `drive_once` action and prove the public result is only advanced, waiting,
  or closed.
- Prove `drive_once` retains no process semantic state and that another process can continue from
  the exact journal.
- Inject compare-and-swap loss after pure computation, observation, and terminal-effect
  verification; prove only pure callbacks repeat and committed evidence is reused.
- Run live and replay through the same state callback identity and canonical output.
- Reject a missing or mismatched selected catalog entry while proving that unrelated process-catalog
  additions do not change an admitted run.
- Compile-fail capability or executor code attempting to construct state output, facts, typed
  failure, transition candidates, or store authority.
- Prove the runtime has no public runner factory, adapter reducer, replay broker, or hook registry.

### External-access audit

- Compile-fail invocation of a registered MFM live capability without its bound
  `AuthorizedReadAccess` or `AuthorizedEnsureAccess`, while proving the lower reusable transport
  remains runtime-agnostic and independently reusable outside MFM.
- Compile-fail direct calls that try to bypass the audited wrapper or substitute raw request data
  for `CommittedRequest`.
- Prove only `NewlyAppended` authorization mints affine authority.
- Prove idempotent, ambiguous, stale, and rejected authorization results mint no authority.
- Reject authority substitution across run, node, capability, `capability_operation_id`, request,
  and effect.
- Compile-fail or reject direct/forged observation construction without a sealed wrapper result.
- Prove the audited wrapper consumes authority even for `DidNotEnter`, permits zero or one boundary
  entry, and performs exactly one independently meaningful application-protocol operation if entry
  occurs.
- Reject a second RPC/HTTP method, independent batch member, response-dependent subcall, hidden
  transport retry, redirect, failover, or provider reselection under one authorization.
- Prove a collection-valued request is one operation only with one indivisible semantic identity
  and shared snapshot/result contract, while a JSON-RPC batch of independently meaningful methods
  is unsupported in the baseline.
- Record `Returned`, `DidNotEnter`, `Indeterminate`, and unmatched authorization.
- Record unrepresentable response failures without retaining raw unsafe bytes.
- Permit only one observation per authorization.
- Require a committed observation before state reduction.
- Reject an observation and its consuming transition in the same batch.
- Reject stale authorization racing read settlement, effect settlement, or closure.
- Prove adaptive read dependencies require another semantic state.
- Cut EVM and Bitcoin balance collection into the specified audited state chains; inject
  cancellation and partial failure after every external operation and prove no sibling invocation
  disappears inside an aggregate result.
- Qualify Bitcoin `scantxoutset "start"` as a read under lost response, cancellation, concurrent
  scan, delayed reissue, bounded work/result, and provider-cost cases; otherwise keep production
  Bitcoin collection unregistered.
- Prove provider/source validation that performs live IO occurs only in post-admission bootstrap
  read states.
- Prove unused and in-flight authorizations do not block semantic closure.
- Admit one late observation for a pre-closure authorization and reject every other post-closure
  append.
- Prove audit records do not change semantic state or authorize effect settlement.
- Verify read retries create distinct authorizations and identify the exact observation consumed
  by the winning transition.

### Typed framework state injection and telemetry

- Prove sealed typed constructors and generic transition/store validation admit no invalid
  authority combination without introducing a pre/post runtime phase.
- Prove operational telemetry cannot change scheduling, transition payloads, state digests,
  closure, or replay; derive committed telemetry from journal records.
- Golden-test canonical plan expansion, injected-node IDs, planner-owned ordering, and exact
  input/output rewiring for direct, nested-child, fan-out, and fan-in operations.
- Prove expansion applies once to authored nodes and never recursively to framework nodes or twice
  through child-operation composition.
- Reject an operation or persisted spec that omits, forges, duplicates, reorders, weakens, or
  bypasses the exact planning-profile result.
- Prove the complete pre-chain consumes and reproduces the protected state's exact input type and
  rewires its producer lineage without a framework-specific frame field or runtime branch.
- Prove a required post-state hides the raw protected output from consumers and public output, and
  that successful closure depends on the effective post-state output.
- Reject cross-run and corrective-run use of a raw protected output as an approved value; resolve
  ordinary source references through the source certified spec to the effective post output.
- Reject a separate framework post-gate around a fact-emitting protected state; prove
  zero protected/intermediate facts plus typed candidate threading to the final outermost post.
  Fail that outer post and prove an inner post emitted nothing.
- Prove injected states use only ordinary pure/read/effect protocols, any injected read uses
  mandatory authorization/observation, and any injected effect is visible and satisfies ordinary
  keyed convergence.
- Reject mutable drive-time policy, arbitrary journal introspection, raw output bypass, runtime
  callbacks, and recursive instrumentation.
- Prove a post-state cannot authorize or reinterpret an already committed effect settlement.
- Prove a fail-stop planning profile injects visible typed success gates and dependency skips
  without a runtime failure branch.

### Keyed effects

- Exactly one pending request per run/node execution.
- Same effect key with another request digest fails closed.
- Resolve the exact immutable executor binding from admission through request, authorization,
  ledger, evidence, restore/failover, and replay; reject a copied or mismatched component tuple.
- Reject an executor deployment whose tenant scope differs from the admitted run.
- `ensure` cannot be called before the pending transition commits.
- Repeated, concurrent, delayed, and post-settlement calls converge.
- Require every returned pending or terminal executor result to carry the exact bound
  delivery-audit frontier; verify empty, attempted, indeterminate, and terminal histories and
  reject a rewritten record or a claimed descendant that omits predecessor history.
- Prove each executor target operation has a positively acknowledged new
  `DeliveryAttemptAuthorized` before entry, permits zero or one entry, and retains a linked outcome
  when it survives; preserve unmatched authorizations as ambiguous.
- Atomically retain every accepted delivery-audit suffix with the MFM observation and reject a
  forked, rewritten, wrongly bound, or invalidly proved frontier; accept a stale ancestor without
  regressing the derived greatest frontier.
- Exhaust the certified delivery-attempt/frontier bound and prove no history is discarded, no
  further target attempt occurs, and the effect safely remains pending when terminal evidence is
  unavailable.
- Inject `target mutated -> executor crash before terminal commit` and require the certified
  downstream convergence behavior.
- Timeout, lookup absence, and provider errors never terminalize the effect.
- Terminal evidence binds key, request, executor binding, external identity, provenance, and
  assurance/finality policy.
- Evidence, output/failure, fact emissions, settlement, and closure are atomic.
- Executor resource ownership survives worker/executor restart, rollback, failover, and split-brain
  attempts.
- Race distinct effects for one executor-owned nonce, UTXO, sequence, or business resource.
- Reject `RunClosed` while any effect remains pending or applied-but-unsettled.
- Fail independent effect A and prove ready effect B may still request by default; under a
  certified fail-stop profile prove B instead becomes dependency-skipped.

### Replay and views

- Replay performs zero live semantic-capability, provider, executor, signer, or domain IO and reads
  only its supplied journal/object and cross-run source dependency bundles, plus the authoritative
  same-store tenant-scoped snapshot when fact-selection completeness must be verified.
- Reject portable verified replay of a run containing fact-selection transitions; expose included
  fact integrity only through the non-authoritative inclusion view.
- Replay recomputes requests and pure reducers from retained inputs/evidence.
- Replay each transition variant and distinguish proof, executor-attestation, and trusted-observer
  assurance.
- Replay every retained executor `Pending` and `Terminal` delivery frontier, including audit-only,
  stale, and post-closure observations; reject an invalid suffix, predecessor, proof, bound,
  binding, effect key, request digest, or fork without consulting the live executor.
- Revalidate every effective-output/evidence-only cross-run source from its complete source
  admission/spec/journal/closure/object/role proof plus exact profile/planner expansion proof;
  reject a missing or downgraded dependency bundle.
- Tampered request, observation, evidence, output, fact, or closure fails verification.
- Status, scheduling, resume, replay, and transition trace consume one verified journal view at one
  loaded journal head. That view reports the fixed semantic closure coordinate separately from any
  later audit-only tail.
- Audit-only tail records leave semantic closure and state digest unchanged.
- No derived view or cache can independently authorize a transition.

### Facts and objects

- Prove a read state authors `FactSelectionRequest` from base inputs without selected facts or a
  fact-query materializer in its `StateFrame`.
- Make the authorization commit order the exact response frontier; admit matching facts committed
  before it and reject later or same-run facts.
- Replay empty, one-result, bounded-many, ordering, limiting, and tie-breaking cases exactly.
- Verify selected facts and objects against producing transitions, descriptors, subjects,
  responses, content identities, store scope, epoch, and frontier.
- Detect an omitted qualifying fact, including an omitted fact from an otherwise selected
  transition, and verify explicit empty results.
- Reject app-supplied replacement of the reserved same-journal fact-selection capability or a
  separate fact store.
- Accept fact-selection completeness only against the authoritative same-store snapshot.
- Restrict fact candidates to other runs in the admitted tenant and reject a state-authored tenant
  selector or cross-tenant fact.
- Require every portable bundle, including a dense global-prefix dump, to report selection
  completeness as unverified; verify included-fact integrity without disclosing an unrelated
  tenant record.
- Test missing, swapped, tampered, or wrong-schema objects.
- Measure history-scan volume and latency.
- If a candidate index exists, property-test scan/index parity and require an incomplete index to
  scan the verified tail or fail closed.
- Prove no fact publication flag, promotion record, dual reader, or compatibility materializer
  exists.
- Prove failed commits admit no object authority.

### Security

- Reject credentials, authorization headers, cookies, secret URLs, private keys, mnemonics, signer
  sessions, raw signed transactions, and bearer payloads.
- Reject raw provider errors, debug representations, and backtraces.
- Reject hashes or fingerprints of credentials, bearer values, secret endpoints, and
  unrepresentable raw responses.
- Admit only reviewed public protocol identities required for correctness, such as an Ethereum
  transaction hash, without admitting the forbidden source payload.
- Bound every provider-controlled result before hashing or retention.
- Derive different run IDs for the same entry point/invocation in different tenants and reject an
  admission candidate whose tenant does not match its `Admit` authority.
- Reject drive, store-backed replay, public read, trace, audit, export, or object access with a
  missing, wrong-purpose, wrong-tenant, wrong-store, or wrong-run `RunAccessAuthority`.
- Require separate source-run authority before a trace or export dereferences cross-run objects,
  and verify every requested object is reachable from the authorized run.
- Verify public status and privileged access-audit export expose only reviewed redacted fields.
- Reject transition/audit/object dereferencing without exact tenant/store/run authorization; prove
  ordinary public APIs expose only status and certified public output by default.

### Correction

- Consume a closed source's spec-resolved effective output in a separately certified corrective
  run.
- Admit a raw protected transition only through an explicit evidence-only input role and prove it
  cannot satisfy an approved value, fact, public output, or equivalence claim.
- Reject missing, unverified, nonterminal, or wrong-source evidence.
- For an entry point claiming at-most-one correction, derive the ordinary invocation identity from
  canonical sources, typed purpose, affected resource, and optional sequence; attach concurrent
  callers and reject arbitrary substitute identities or changed root material.
- Prove legitimate installments require distinct typed sequence values, while an implementation or
  contract version bump alone cannot duplicate a correction.
- Prove source closure creates no hidden correction obligation.
- Deduplicate a controller-launched correction through the same domain-derived admission identity.
- Require typed equivalence evidence before publishing `compensated`.

### EVM qualification

- Keep transaction mutation unregistered before a keyed executor qualifies.
- Test signer-account coordination across every process/deployment/actor and
  same-key/different-request rejection.
- Test restart, response loss, already-known transactions, rebroadcast, replacement policy,
  success, revert, finality, and reorganization.
- Audit MFM-to-executor accesses directly and require separately labeled linked executor evidence
  for every executor-internal target-operation attempt.
- Prove raw signed bytes remain outside MFM persisted and public surfaces.

### Store parity and schema

- Freeze canonical bytes and derived identities for every record, transition body, logical key,
  binding, manifest, reference, digest preimage, absent/empty case, and enum tag before backend
  implementation diverges.
- Run the same positive and negative golden-vector corpus through memory, PostgreSQL, runtime,
  replay, trace export, and the durable reference executor.
- Inject failure after every database operation in each legal batch shape.
- Prove Postgres and memory all-or-nothing parity.
- Verify genesis and predecessor-linked candidate/record/commit digests, complete object bindings,
  append idempotency conflicts, and store-order behavior.
- Verify database constraints and no-update/no-delete/no-truncate enforcement.
- Fork a backup/restore clone at one head and prove only one fenced writable lineage may append;
  reject stale-primary, sibling-promotion, published-suffix rollback, and history merge.
- Prove destructive reset generates a never-before-used scope and fresh epoch, while verified
  non-rollback restore preserves both; reject same-scope reset even with a new epoch.
- Treat ambiguous `COMMIT` as `OutcomeUnknown` and prove it never mints access authority.
- Prove every semantic live probe occurs after `RunAdmitted`.
- Qualify the durable reference executor, including destination mutation followed by executor crash,
  tombstone retention, original-binding upgrade routing, restore, rollback, and split brain, before
  freezing its generic binding/evidence schemas.
- Reject legacy event/projection/saga schemas explicitly.
- Prove old compatibility paths and dual writers do not exist.

## Pre-Cutover Gates and Work Packages

The ordered commits below are coherent landing boundaries, not sufficient implementation tasks.
The vertical cutover does not begin until these gates close in order:

1. **Contract decisions:** accept the per-operation audit unit, no baseline transport batch,
   graph-derived forward-failure semantics, domain-derived correction identity, exact executor
   binding, same-tenant cross-run scope, same-store-only fact completeness, and purpose-bound run
   access.
2. **Production prototypes:** express the EVM and Bitcoin reads as the specified typed state chains
   and qualify one durable reference executor against a convergence-safe destination.
3. **Identity and schema freeze:** finalize node-occurrence derivation, terminal/dependency
   contracts, every canonical persisted schema, and the complete golden-vector corpus.
4. **Implementation package plan:** assign crate ownership, dependency order, deletion checkpoints,
   focused verification, and final integration responsibility for every package below.

The implementation packages are:

```text
A  surviving committed-journal / private-fold / verified-view ownership
B  canonical schema, identity, codec, digest, and golden-vector corpus
C  authored-program expansion, planning profile, certification, and terminal/dependency contracts
D  journal records, sealed append authority, memory store, and PostgreSQL parity
E  closed StateExecution catalog and stateless drive_once
F  per-operation read audit wrappers and production read graph migration
G  keyed effect request, executor binding, bounded delivery frontier, terminal evidence, and
   reference executor
H  replay, cross-run sources, same-store fact completeness, and portable inclusion-only reporting
I  app/CLI/REST run-access authority, status, privileged trace/audit, and public-output defaults
J  deletion of attempts, phases, saga, lanes, projections, adapter lifecycle/reducers, old schemas,
   and documentation
```

Packages B and the two prototypes produce contracts consumed by the later packages. Packages C
through J may be developed and reviewed behind the cutover branch boundary, but they do not ship
as a parallel lifecycle, dual writer, compatibility reader, or partially selectable runtime. The
final producer/consumer/schema switch remains coherent and deletes the old path in the same landing.
Package D owns persisted authorization/observation legality. Package E owns the shared sealed
affine authority, committed-observation types, and generic audited wrapper. Packages F and G add
only read- and effect-specific orchestration over that one boundary.

## Ordered Logical Commits

1. **`use one committed journal across store runtime and replay`**

   Replace copied committed-stream, projection, artifact, runtime-history, and replay-map authority
   with one native committed journal, one private fold, and one opaque verified view. Preserve the
   current persisted behavior while deleting superseded in-memory representations. Land only APIs
   that survive unchanged as package A; do not encode old attempt, saga, phase-ledger, or projection
   concepts into the new public boundary.

2. **`replace run lifecycle with complete audited transitions`**

   Perform one inseparable vertical cutover across program, spec, certification, events, store,
   runtime, replay, app, binaries, Postgres, tests, and documentation. Introduce complete transition
   records, per-operation external-access authorization/observation, affine access authority, keyed
   effect requests with immutable executor bindings, the durable reference executor,
   tenant-scoped admission and purpose-bound run access, domain-derived correction identity, the
   closed three-case `StateExecution`, `drive_once`, state/capability catalogs,
   certified planning-time framework state injection, semantic closure with audit tails, and
   transition facts. Delete worker attempts, custom runners, adapter-owned lifecycle/reducers,
   replay brokers, phase ledgers, saga/manual resolution, resource lanes, universal projections,
   synthetic completion/retention, physical fact-projection authority, old events, and every
   compatibility path. Cut fact storage,
   selection, completeness replay, and transition emission over in this same commit; use the
   reserved audited read capability and its private history scan at the authorization frontier
   with no dual fact reader.

3. **`qualify evm writes through a durable keyed executor`**

   Add the reviewed wallet/relayer contract and conformance suite, then enable EVM transaction
   registration. Until this commit is possible, EVM mutation remains deliberately unavailable.

Each commit updates its code, tests, design and architecture contracts, persisted/public surface
inventory, and relevant API documentation. Inseparable producer/consumer/schema changes remain in
the same commit. No intermediate compatibility or parallel lifecycle path is allowed.

## Alternatives Rejected

### Store only batch results

Rejected. A batch is an atomicity envelope, not the semantic unit. Each state transition must keep
its exact before/input/result/output/after trace and individual identity.

### Persist full run snapshots before and after every state

Rejected. Exact input manifests, binding deltas, before/after state digests, and deterministic
folding provide complete traceability without duplicating the entire run on every transition.

### Keep generic worker attempts for audit

Rejected. Worker attempts mix process ownership, scheduling, recovery, and semantic state. The
required audit primitive is only the external capability authorization/observation pair.

### Keep extensible runner and adapter lifecycles

Rejected. New state semantics should extend typed state contracts, requests, and capabilities, not
the kernel's lifecycle algebra. Custom runners, runner-owned event payloads, adapter reducers, and
replay implementations let equivalent states take different execution paths and move semantic
computation outside the state contract.

### Allow several independent or adaptive requests inside one read state

Rejected. It requires request scheduling, partial evidence sets, and observation-selection policy
inside runtime or adapters. One read state owns one typed request and accepts one committed
observation for one independently meaningful application-protocol operation. Response-dependent
or independent requests are explicit state chains. A later transport-batching design may preserve
one authorization and outcome identity per member, but cannot collapse them into one audited
operation.

### Keep a generic variable-cardinality evidence bag

Rejected. Required evidence cardinality and role become implicit and substitution-prone. Read and
effect evidence use their exact consumed-observation fields; other domain evidence is a declared
typed output or fact slot.

### Record only successful external results

Rejected. Unmatched authorizations, pre-entry rejection, transport ambiguity, and unrepresentable
returns are precisely the failures that typed state results cannot capture and are required audit
history.

### Claim exact physical-call audit from local records

Rejected as impossible without a participating remote gateway or broker. A local append before a
call can exist when no call occurred; an append after a call can be lost after the call occurred.
The honest generic guarantee is durable authorization before every MFM-controlled invocation.

### Allow hidden retries below one audit record

Rejected. It makes physical access count and ambiguity invisible. Every in-process
application-protocol operation needs its own authorization and bound affine authority.

### Keep a sealed same-transition framework envelope

Rejected. Named prechecked/evaluated/postchecked/commit-ready types add another runtime protocol but
cannot safely provide IO, retries, independent traceability, or semantic extensibility. Structural
authority is already excluded by sealed constructors and unavoidable transition/store predicates.
Semantic pre/post behavior reuses ordinary typed states through exact-profile planning expansion.

### Implement framework safety as runtime pre/post hooks

Rejected. Hooks are absent from the certified graph and journal, can vary across live/resume/replay,
and create another extensible execution lifecycle. Structural validity is excluded by sealed types
after unavoidable boundary verification. Separately traceable semantic policy is deterministic
certified plan expansion over ordinary states.

### Let each operation opt into mandatory framework wrappers

Rejected as the source of a platform guarantee. Operations are an authoring surface and may use
typed state combinators. Production entry points pin an exact planning profile, and the framework
plan expander plus certifier enforce it. An operation cannot omit or bypass that profile.

### Inject telemetry states around every user state

Rejected. It makes observation part of semantic progress, can block closure, recursively
instruments itself, and greatly expands the graph and journal. Ordinary telemetry derives from
driver spans and committed journal records. Anything whose delivery or value must affect the run
is explicit typed workflow behavior, not telemetry.

### Keep the current phase ledger and add transition manifests

Rejected. It would add the desired trace while retaining every duplicated recovery and projection
model. The phase ledger, generic attempts, and saga machinery are the carrying cost being removed.

### Support both observation-only and repeat-exact recovery

Rejected. Observation-only can wedge after request commit and pre-dispatch crash. Two recovery
modes recreate first-versus-recovery dispatch permits. One keyed-convergent executor contract is
smaller and provides actual recoverability.

### Make exact transport bytes the generic idempotency contract

Rejected. Exact bytes can execute twice against a non-idempotent target and are unnecessary for a
destination enforcing a semantic operation key. Transport bytes may also include credentials or
bearer material.

### Use `active_effect_keys` as another authoritative table

Rejected. A local table or journal micro-fold can order MFM admissions but cannot revoke an already
issued delayed call or reserve a resource against another actor. Cross-effect resource ownership
belongs to the executor or destination; keeping a core lane would add another lifecycle without
establishing external quiescence.

### Remove every physical index

Rejected. Ordinary B-tree indexes and rebuildable candidate indexes can improve performance without
becoming semantic truth. The architectural rule is one authority, not one physical table.

### Persist raw provider bodies for audit

Rejected. External systems may echo credentials, bearer tokens, private endpoints, and unbounded
data. Persist reviewed typed results or a safe unrepresentable/indeterminate failure only.

### Make the certified core read-only

Rejected. Writes remain a platform goal. The keyed executor contract keeps one honest mutation
primitive without pushing recovery policy into every workflow.

### Retain a universal first-failure forward-effect fence

Rejected as a kernel invariant. A node failure does not prove that every independent effect has
become semantically invalid. Exact planning profiles may inject typed success gates when fail-stop
behavior is required. Restoring the former dynamic fence would add a run-wide scheduler policy and
failure-domain algebra to the thin runtime; preserving its concurrent-then-fence behavior is outside
this baseline rather than silently approximated.

### Keep generic saga under weaker names

Rejected. Obligation derivation, remediation ordering, manual terminalization, and compensation
claims remain domain semantics. Explicit corrective runs are smaller and make no automatic
equivalence claim.

## Normative Guarantee Change Ledger

This table states the contract change relative to the current authoritative design. “Maintained”
means the semantic guarantee survives, not that its current public type or lifecycle machinery
survives.

| Disposition | Guarantees |
| --- | --- |
| Maintained | First-class certified mutation with exactly one mutation authority per effect state; behavioral interruption recovery; append-only history; per-append atomicity; deterministic evidence-only replay; pure state logic with no ambient semantic IO; stable effect identity; canonical/content-addressed and no-secret persistence. |
| Added | Complete transition before/input/evidence/result/output/after lineage; one authorization per independently meaningful external operation before access; one observation per surviving wrapper result; keyed-convergent redelivery; stable tenant-scoped admission identity; immutable semantic closure; exact capability/executor binding; domain-derived at-most-once identity for correction entry points that claim it; purpose-bound run access and privileged trace/object access. |
| Removed | Generic saga/remediation obligations; reverse compensation ordering; manual terminalization; worker-attempt history; phase ledgers; resource lanes; custom runner and adapter-owned lifecycle/reducer machinery; runtime pre/post hooks; universal projection authority; support for non-convergent writes; compatibility readers and dual writers. |
| Delegated | Destination delivery and convergence, nonce/UTXO/sequence/resource ownership, and executor-internal delivery-attempt evidence move to the exact certified executor/domain binding. They are not a second MFM run-state model. |
| Changed or restricted | Independent ready effects may begin after another node fails unless the certified graph/profile gates them; semantic closure may receive audit-only observations for pre-closure authorizations; unresolved effects may remain pending indefinitely; finite executor delivery-record/frontier bounds forbid another target attempt and can force permanent pending, with no baseline pagination; EVM writes remain unavailable until qualification; cross-run sources and facts are same-tenant; fact-selection completeness is same-store only and portable bundles are inclusion-only; complete transition and audit traces are privileged rather than ordinary public output. |

## Decision Summary

The target core is an auditable, event-sourced typed state machine:

- one stateless `drive_once` interpreter over closed pure, read, and effect contracts;
- only `Unstarted`, `AwaitingEffect`, and `Terminal` semantic node phases;
- one complete state-transition record per semantic change;
- one atomic commit envelope around each transition and its objects;
- exact input/output/fact/evidence lineage with no generic evidence bag, retained indefinitely at
  first;
- one durable external-access authorization before every independently meaningful direct
  MFM-controlled application-protocol operation, with no hidden multi-operation read capability;
- executor-internal target attempts captured separately through immutable evidence under the exact
  bound executor contract and deployment;
- one mandatory immutable observation for every surviving wrapper result, recording returned,
  pre-entry, indeterminate, or unrepresentable outcomes;
- no claim that a local audit record proves exact physical remote delivery;
- one stable pending effect request and one immutable executor binding backed by a reviewed
  keyed-convergence and downstream convergence mechanism;
- verifier-backed terminal settlement;
- one typed read request and one state-accepted committed observation per external-operation read
  occurrence;
- one compiled state catalog, one live capability catalog, exact per-run selected-entry manifests,
  and the same state callbacks in replay;
- same-tenant cross-run fact selection as an ordinary audited read through one non-overridable
  same-journal capability, with no pre-state query materializer or fact projection authority,
  same-store-only completeness, and inclusion-only portable inspection;
- structural invariants excluded by sealed typed construction and unavoidable trust-boundary
  validation, with no framework lifecycle in runtime;
- semantic framework pre/post behavior injected as ordinary typed states during deterministic plan
  expansion and independently verified by certification;
- spec-resolved cross-run source roles that cannot bypass an effective post output;
- one immutable tenant scope per admitted run and transient purpose-bound authority at every run
  access surface;
- one fenced writable journal lineage for each store identity;
- operational telemetry derived without semantic authority;
- semantic closure that still accepts constrained late audit observations;
- independent post-failure effects governed by certified graph/profile dependencies rather than a
  run-wide runtime fence;
- one private fold and one opaque verified run view;
- facts, status, recovery, replay, public output, and analysis derived from the journal;
- no generic worker attempts, custom runner or adapter-owned lifecycle/reducer, replay broker,
  phase ledger, saga, manual resolution, resource lanes, runtime hook chain, or universal
  projection;
- explicit typed corrective runs with domain-derived admission identity for domain remediation; and
- privileged tenant/run-authorized transition, audit, object, and portable-export surfaces with
  public-output-only dereferencing by default.

If data affects a semantic transition, it must appear in that transition's certified inputs,
accepted observations, outputs, facts, evidence, or before/after anchors. If MFM authorizes a live
external access, that authorization must be durably visible before the call. Anything else is
operational telemetry and cannot authorize runtime state.
