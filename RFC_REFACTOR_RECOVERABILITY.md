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
3. The audited capability performs at most one boundary invocation and no hidden retry.
4. MFM durably records the one `ExternalAccessObserved` outcome whenever the wrapper result
   survives; only a crash or unresolved append can leave it absent.
5. A semantic transition may consume only a committed, reviewed observation.

This protocol proves:

```text
MFM-controlled capability invocation
    implies
a matching durable authorization already existed
```

It deliberately cannot prove the converse. A process can crash after authorization and before
entering the external boundary. An authorization without an observation is therefore permanently
`CrashAmbiguous`: zero or one physical invocation may have occurred.

Writes use one stronger and smaller recovery primitive:

```text
ensure(committed effect key, immutable semantic request)
    -> Pending | Terminal(typed evidence)
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
capability invocation.

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
- Record every MFM-controlled external read or write capability authorization before access.
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
- Keep universal framework enforcement in a sealed same-transition envelope, and specify the
  activation contract for future separately traceable semantic pre/post graph expansion.
- Preserve atomic appends, content addressing, canonical JSON, float-free hashed structures, and
  no-secret persisted surfaces.
- Replace generic saga semantics with explicit typed corrective runs.
- Minimize concepts, public types, duplicated folds, future change sites, and lines of code.

## Non-Goals

- Proving that a durable authorization corresponds to exactly one physical network request.
- Auditing the journal store's own database calls, which would create infinite regress.
- Auditing DNS, TLS packets, kernel syscalls, or executor-internal provider calls as MFM journal
  records.
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
- Implementing semantic framework graph expansion before a concrete mandatory rule exists.
- Runtime-loaded native libraries, WASM states, or remote state-execution ABIs in this cutover.
- Cross-store import of run-source authority.
- Backward compatibility with current event, projection, attempt, saga, or store schemas.

## Material Uncertainties

| Choice or assumption | Why it is uncertain | Consequence if wrong | Resolution or validation |
| --- | --- | --- | --- |
| One audited access means one certified capability-boundary invocation. | An in-process capability or remote executor may perform several HTTP/RPC exchanges internally. | MFM's run journal would not enumerate every downstream physical request. | Forbid hidden retries and redirects in in-process capabilities. Require linked executor-supplied audit evidence when executor-internal calls are in scope. |
| A durable authorization is acceptable as the honest pre-call audit fact. | No local database protocol can atomically prove that a remote boundary was physically entered. | Consumers could incorrectly read authorization as proof that a request was sent. | Name and document the record as authorization, expose unmatched records as `CrashAmbiguous`, and never claim exact physical delivery. |
| Reviewed typed results and safe failure summaries provide enough audit detail. | Raw provider bodies or error chains may contain useful diagnostics as well as secrets. | Redaction may omit forensic detail; retaining raw material may violate the no-secret invariant. | Define bounded public result/failure schemas per capability and adversarially test credential, bearer, and low-entropy-secret leakage. |
| Audit correlation and retention are acceptable in the run journal. | Even redacted access order, frequency, source choice, and wall-clock metadata may reveal tenant behavior or provider incidents. | Indefinite immutable retention could conflict with privacy, erasure, or least-access requirements. | Classify every audit field, omit source and precise time by default, restrict audit export, and define the production retention policy before rollout. If deletion is mandatory, resolve the journal-integrity and archival contract in a follow-up RFC rather than silently weakening this trace. |
| Semantic closure may be followed by audit-only observations. | Current terminal models often prohibit every later run append. | A stale call returning after closure could not be recorded, or one crashed call could block closure forever. | Make closure absorbing for semantic transitions and new authorizations while allowing one observation for a pre-closure unmatched authorization. |
| Two durable audit appends per live access are affordable. | High-frequency reads can create substantial latency and journal volume. | The audit primitive may dominate execution cost. | Implement correctness first, measure representative workloads, and optimize only without hiding individual authorization identities or weakening affine access authority. |
| Strict audit availability may gate every live access. | Durable authorization must succeed before the capability becomes callable. | A journal outage becomes a provider-access outage rather than degraded audit coverage. | Keep the protocol fail closed and measure the availability budget. A deployment requiring a best-effort bypass does not implement this contract and cannot register the audited live capability. |
| Database HA can provide non-rollback exclusive writer fencing for one preserved store identity. | A restored clone may otherwise accept writes while an old or sibling primary is still writable. | Identical run/store coordinates could name divergent journals, invalidating facts, cross-run refs, and effect recovery. | Qualify the deployment fence, require proof of the unique latest prefix before promotion, test stale-primary/restore split brain, and fail closed if a published suffix may be lost. |
| Intended mutation executors can provide permanent keyed convergence. | Some systems have finite idempotency windows or no client-chosen identity. | A delayed call could create another mutation or cost-bearing operation. | Inventory executors and certify permanent destination-native convergence or a keyed executor with its own reviewed downstream convergence proof. Unsupported effect states remain unregistered. |
| Executor-owned delivery state is acceptable as an irreducible external authority. | The goal is to remove secondary run-state stores, but an executor ledger still does not create destination convergence by itself. | Mistaking the ledger for a generic idempotency proof could duplicate a downstream mutation after `target applied -> executor crash`. | Treat the ledger as external-delivery authority only and require an independently reviewed downstream convergence mechanism, permanent anti-rollback bindings, and disaster-recovery tests. |
| Cross-effect resource coordination can leave the kernel. | Different effect keys may still compete for one nonce, UTXO, sequence, inventory item, or business resource. | Removing resource lanes without a replacement owner could admit conflicting external operations. | Require the executor/destination to provide exclusive ownership, one shared durable coordinator, or atomic domain preconditions. Keep an effect state unregistered until that ownership is concrete. |
| One typed semantic request and one accepted observation are sufficient per read state. | Current provider workflows may contain several independent or response-dependent calls. | A hidden multi-call capability implementation would recreate orchestration and untracked semantic choices outside the state graph. | Inventory every read implementation. Use one reviewed reusable typed batch only when it is genuinely one capability request with complete member evidence; split adaptive or independent work into typed state chains. |
| Read and effect request authorship can be total over certified typed inputs. | Existing planners may accept weak types and report domain errors before producing a request. | A generic pre-call runner-failure path would reintroduce another lifecycle. | Move fallible validation into an upstream pure state that produces a stronger type and property-test request totality. If a legitimate case remains, specify one closed local typed-failure transition rather than generic runner errors. |
| A future concrete semantic pre/post rule can use deterministic graph expansion without runtime hooks. | A postcondition may need to gate downstream visibility, and current operations may not expose all required typed lineage. | A callback that merely runs after a state could be bypassed, observe the wrong transition, or falsely claim enforcement. | Keep expansion out of this baseline. A follow-up activation must name/type-check the rule, make transform identity and policy hash-defining, bind permits/effective outputs, and pass the contract below. |
| Future multiple mandatory rules compose with one canonical typed onion order. | Rules may transform or gate the same input/output types. | Rule order could change semantics or make effective handles incompatible. | Before activation, define rule compatibility constraints and golden-test every supported composition; certification rejects an incompatible profile. |
| Future production entry points can own the minimum certification profile. | Library consumers may assemble MFM with different mandatory policies. | An operation or weaker assembly could omit a guarantee that the framework claims globally. | The activation RFC must pin and publish one minimum profile per entry point, bind it into the certified spec/admission root, and reject weaker admission. |
| Required post-states need only explicit typed inputs and the protected typed output. | A proposed policy may ask to inspect arbitrary journal or audit internals. | A privileged post context would recreate a second runtime API and couple policy to storage representation. | Inventory concrete postconditions. Keep observation/effect-evidence verification in the producing state and reject generic journal-inspecting post hooks. |
| The generic evidence-reference bag can be removed completely. | Existing states may retain auxiliary evidence outside outputs or facts. | Migration may lose trace material or reintroduce an untyped escape hatch. | Inventory every evidence producer and consumer; convert each to the exact access-observation reference, an explicit typed output/fact slot, or delete it. |
| Canonical expansion can occur once before final node identities are frozen. | Child-operation composition may currently lower graphs in several stages. | Framework nodes could be duplicated or acquire unstable identities, changing graph hashes and effect keys. | Freeze expansion/identity ordering and golden-test direct, nested-child, fan-out, and fan-in construction while certification independently revalidates the result. |
| Operational telemetry remains non-semantic. | Some deployments may require durable compliance evidence or guaranteed delivery to an external sink. | Treating such delivery as a best-effort hook would overstate the guarantee; treating ordinary telemetry as states would bloat and couple semantics. | Derive ordinary telemetry from journal/driver observations without authority. Model compliance evidence that affects decisions as explicit typed states, and guaranteed external delivery as a qualified effect. |
| Compiled Rust registration is sufficient for extensibility. | Future consumers may request WASM, dynamic-library, or remote state implementations. | The proposed state catalog does not define a stable ABI or isolation boundary for them. | Keep this cutover to compiled, certified registrations. Design dynamic execution as a separate authority and isolation contract. |
| Full transition and artifact retention is acceptable initially. | Complete future analysis requires the referenced bytes, not only their hashes. | Indefinite retention can produce material storage growth. | Measure expected volume. Design garbage collection only after a complete dependency-closure and archival contract exists. |
| A same-journal history scan is sufficient for the initial reserved fact-selection capability. | Fact volume and latency objectives are not defined. | Cross-run selection can become unbounded, and its two audit appends may be material. | Locate every fact-bearing transition through a validated routing column and batch-verify it initially. Measure full audited requests, and add only a rebuildable, watermarked candidate index when required. |
| Replay has authoritative fact history through every pinned frontier. | A portable bundle may otherwise contain only selected facts or omit a whole fact-bearing commit. | It can verify selected values but cannot detect omitted qualifying facts. | Require the contiguous global commit/record prefix through the frontier or an authenticated census/completeness export; downgrade every unauthenticated subset instead of claiming complete replay. |
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
: One invocation of a certified live read capability or keyed effect executor at the MFM-controlled
capability boundary.

**External access authorization**
: A durable record that MFM authorized zero or one invocation of the identified capability with the
identified public request.

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

**Framework graph expansion**
: The deferred activation contract for a deterministic, hash-defining certification step that may
surround eligible authored nodes with ordinary typed framework states. It is not implemented by
this baseline and can never be a runtime callback or operation-author convention.

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
selected for an admitted run is retained in that run's `RunAdmitted` root record.

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
| `ExternalAccessAuthorized` | Authorizes zero or one audited capability invocation; changes no semantic state. |
| `ExternalAccessObserved` | Records a reviewed capability return or safe failure; changes no semantic state. |
| `RunClosed` | Structurally seals semantic state at a verified terminal transition. |

Facts, cells, public output, effect requests, and typed failures are fields or referenced values of
`StateTransitionCommitted`, not independent lifecycle state machines.

## Run Root Record

The first commit retains the complete root lineage:

```text
RunAdmitted {
    version,
    run_id,
    invocation_identity,
    operation_id,
    operation_contract_ref,
    spec_hash,
    certified_spec_ref,
    certificate_ref,
    state_implementation_manifest_ref,
    capability_implementation_manifest_ref,
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
    operation_id,                      # stable operation namespace, not contract version
    invocation_identity,
)
```

`invocation_identity` is a canonical non-secret logical-start idempotency key. The store permits
one admission for `(store_scope_id, operation_id, invocation_identity)`. Repeating the same
identity with the exact root candidate reloads/attaches to the existing run; changing spec,
contract, config, seeds, context, or certificate conflicts rather than creating another run.
`store_scope_id` is a never-reused lineage namespace: a destructive reset must generate a fresh
scope and epoch before it can admit another run.

A lost admission acknowledgement is resolved by the same derived `run_id` and
`append_request_id`. It never generates a fresh run identity. This is required because a fresh run
would also derive fresh effect keys and could duplicate a business mutation despite perfect
per-effect executor convergence.

If the original append identity was lost with the process, `AdmitRun` still looks up the derived
run/admission logical key. Exact root content returns `ExistingSame` and attaches without another
commit; different root content returns `Conflict`. Only a directly observed first commit returns
`NewlyAdmitted`, and connection ambiguity returns `OutcomeUnknown`.

The manifests enumerate every typed root slot, schema and semantic type, content digest, object
evidence, context constraint, and exact state/capability implementation selected for this graph.
They contain only entries required by the certified spec. Capability entries identify reviewed
contracts and implementations but contain no credentials, endpoints, or secret-derived
identifiers. `initial_bindings` are the exact cells available before the first state transition.
Any cross-run root binding must use the spec-resolved effective-output or evidence-only source role;
copying a raw object reference cannot bypass source lineage or framework post-gating.

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
        logical_source_occurrence_id,
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
        logical_source_occurrence_id,
        raw_transition_ref,
        raw_result_or_evidence_ref,
        certified_evidence_role_ref,
        source_closure_ref,
    }
```

`EffectiveOutputSource` resolves the logical source through the source certified spec. If a later
spec activates framework expansion, that resolution reaches its final effective post output.
`EvidenceOnlySource` is legal only for a destination slot whose certified contract explicitly
accepts that correction-evidence role. It cannot satisfy an ordinary domain value, output, fact,
public output, or equivalence claim. Producer-bound semantic types cannot be supplied as anonymous
config/context merely by copying identical bytes. Every source run is closed before destination
admission, making the transitive source dependency graph acyclic. In this baseline,
`source_store_scope_id` and `source_store_epoch` must equal the destination store; cross-store
source authority is rejected rather than fetched through ambient IO.

Objects first produced by admission use `ThisAdmission(slot)` or logical seed/config identities,
never the future `RunAdmitted` record coordinate. `certified_spec_ref` is the retained canonical
spec object whose content digest equals `spec_hash`.

Admission verifies the certificate, invocation identity, operation/spec relationship, exact
selected state/capability implementation manifests, every cross-run source
closure/lineage/role/effective-output resolution, all object bytes/evidence, the explicit genesis
predecessor, and the canonical initial semantic-state digest. Adding an unrelated entry to a
process catalog cannot change an existing run. The first transition can therefore prove every root
input without consulting current configuration or deployment state.

## Complete State Transition Record

A transition is approximately:

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
            executor_contract_ref,
        }
      | EffectSettled {
            request_transition_ref,
            request_input_manifest_ref,
            consumed_terminal_observation_ref,
            settlement,
        }
      | DependencySkipped {
            blocking_sources,
            skip_rule_ref,
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

Its logical identity is `(run_id, certified_node_occurrence_id, slot)`, where `slot` is
`Request` only for `EffectRequested` and `Settlement` for every settled or skipped variant. The
store permits at most one record in each legal slot, requires `EffectSettled` to reference the one
request slot, and rejects a second or mismatched settlement.

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
successful typed output, not a failed node with a cell. `DependencySkipped` is a persisted
transition and names every blocking source transition/output or certified run-failure source, its
closed reason, and the certified skip rule; it produces no output or fact.

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
identity, capability or executor contract, and certified failure and skip policy. Replay verifies
each body variant under that exact contract.

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

If a value affected a state result but is absent from the certified input manifest or accepted
evidence, replay must reject the transition.

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
  certified spec, certificate, selected-implementation, config, seed, context,
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
- unresolved effect node slots, keys, request digests, and executor contracts; and
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

An audited access is one entry into an explicit MFM-controlled certified capability API. An
in-process capability may initiate at most one independent application-protocol operation under
that entry and must not retry, redirect, fail over, or reselect a provider invisibly. One read state
authors exactly one typed semantic request. A deterministic bounded batch may be that one request
only when the capability contract defines the complete reusable batch, its observation accounts
for every member, and no semantically dependent hidden subcall is omitted. Independent or adaptive
requests are separate typed states.

Kernel/TCP retransmission is not another semantic capability entry and cannot be counted from
local Rust typestate. The capability's read or keyed-convergence contract must make such
transport-level duplication safe. An HTTP/RPC library retry that can create another independent
application request is not a transport detail and requires a fresh authorization.

A remote keyed executor may make internal target calls. MFM audits its call to that executor, not
the executor's internal network exchanges. A product requiring those exchanges in the MFM trace
must require linked, replay-verifiable executor audit evidence.

Every new live invocation requires this protocol. “Obtain audited evidence” is conditional only in
the following senses: pure and skipped states perform no live access, and a read/effect state may
reuse sufficient committed evidence instead of calling again. Once runtime decides to invoke a
live capability, durable authorization before entry and durable observation for every surviving
wrapper result are mandatory. There is no unaudited or best-effort evidence path.

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

    capability_contract_ref,
    admitted_implementation_ref,
    safe_classifier_contract_ref,
    operation_id,
    request_ref,
    reviewed_source_alias?,
}
```

The record coordinate is the access identity. There is no generic worker `AttemptId`, attempt
number, owner, lease, epoch, or retry counter.

The semantic anchor binds a read to the exact verified state view from which its typed request was
authored, including the initial view where no predecessor state transition exists. An effect
authorization additionally binds the already committed request transition.

`reviewed_source_alias` is an optional closed, non-secret configuration identity. It is never an
endpoint, URL, hostname, filesystem path, provider-supplied identifier, or arbitrary string.

Under the append transaction, the store requires:

- the anchored journal head and state digest are still current;
- a read node remains ready under the same certified request;
- an effect remains `AwaitingEffect` for the exact request transition, key, and digest;
- capability, operation, request, and executor contracts still match certified authority; and
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

> MFM durably authorized zero or one invocation of this audited capability boundary using this
> reviewed public request. The record does not prove that boundary entry or remote receipt
> occurred.

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
capability_contract_ref
operation_id
request_ref
committed effect request                       # ensure only
```

An idempotently found record, reload, stale response, commit-then-error, or lost acknowledgement
mints no authority. A later physical call requires a fresh authorization append identity even when
the semantic read request or effect request is unchanged.

The audited wrapper consumes and destroys the affine authority when it accepts the invocation,
before local validation or external boundary entry. It never returns that authority. It may then
perform at most one audited invocation and cannot retry, redirect, fail over, or pair the authority
with another request. A local rejection after acceptance produces `DidNotEnter`.

Only this wrapper can construct the sealed `UncommittedAccessObservation` accepted by
`ObserveExternalAccess`. Schema-valid result bytes, a record reference, or an independently
constructed request cannot mint an observation.

This establishes the one-way audit invariant:

```text
physical MFM-controlled capability invocation
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
influence state. “No observation” represents process loss or unresolved append ambiguity, not an
optional logging policy.

On load, runtime exposes only a sealed `CommittedAccessObservation` that verifies the containing
commit, authorization, run and node, semantic anchor, capability and admitted implementation,
operation, public request digest, result schema, and observation role. A semantic transition may
reference it only from a strictly later commit.

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
  input-manifest reference, capability, operation, request, and response schema.
- `EffectSettled` accepts only `Returned(Terminal)` for its exact request transition and executor.
- `Returned(Pending)`, unmatched authorization, and `CrashAmbiguous` are never settlement evidence.
- `DidNotEnter` or `Indeterminate` may be consumed only by an explicitly certified pure read-failure
  policy; retry count, elapsed time, and worker behavior are not policy inputs.
- Each observation can justify at most one committed semantic transition. Reuse after that point is
  through the transition's typed output, not raw audit evidence.

The store rejects cross-run, cross-node, cross-capability, cross-operation, cross-request,
wrong-schema, stale-authority, and already-consumed substitution.

### Read workflow

```text
Ready node
  -> author one deterministic typed read request
  -> append ExternalAccessAuthorized
  -> audited wrapper consumes AuthorizedReadAccess
  -> perform at most one capability invocation
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
           executor_contract_ref,
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
       Returned(Pending)
       | Returned(Terminal(evidence))
       | DidNotEnter
       | Indeterminate
     )
```

`Pending`, `DidNotEnter`, `Indeterminate`, and unmatched authorizations leave the node
`AwaitingEffect`.

`Returned(Terminal(evidence))` becomes a committed observation. The state-owned pure verifier
checks it against the original inputs, immutable request, certified executor contract, provenance,
external identity, and assurance/finality policy. Only then may a semantic transition settle the
node.

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

- exists only because zero or one capability invocation may occur;
- contains no worker, owner, lease, epoch, attempt number, retry policy, or interruption;
- does not change node or run state;
- does not block scheduling or terminality;
- does not become effect identity;
- is never taken over, rewritten, or abandoned; and
- pairs with at most one immutable observation.

Pure computation, worker execution, scheduling, and process takeover have no persisted attempt
events. One read or pending-effect evaluation may produce several audit pairs across retries, but
each live invocation has its own pair and exactly one later semantic transition names the one
observation it accepts.

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
    Pending,
    Terminal(Evidence),
}
```

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
    executor_namespace,
    store_scope_id,
    run_id,
    certified_node_occurrence_id,
    executor_contract_ref,
)
```

`executor_contract_ref` is the one reviewed keyed-execution contract identity. The
`EffectRequested` record, effect-key derivation, `EnsureEffect` authorization's
`capability_contract_ref`, executor ledger binding, terminal evidence, and replay must all use that
same identity. State-owned semantic request rules remain in `state_contract_ref`; there is no
second ambiguous `effect_contract_ref`.

The key deliberately excludes request bytes. Its one immutable request digest is bound by the
pending transition, and reuse with another digest fails closed. `store_scope_id` remains stable
across a verified non-rollback backup restoration of the same lineage; restoring a resumable store
under a new scope is forbidden. A destructive reset must use a never-before-used
`store_scope_id` and a fresh `store_epoch`, so it cannot recreate a prior run or effect key.
Concurrent clones must share the same executor ledger and deployment fence or only one may drive
effects. `executor_namespace` authenticates tenant and durable ledger generation. Contract
upgrades must route an existing pending key back to its original non-rolled-back ledger; inability
to do so leaves the effect blocked rather than treating the key as new.

Executor evidence is wrapped by a kernel envelope:

```text
TerminalEffectEvidence {
    executor_namespace,
    executor_contract_ref,
    effect_key,
    request_digest,
    external_operation_identity,
    terminal_outcome,
    assurance_policy_ref,
    proof_basis:
        SelfAuthenticatingProof
      | ExecutorAttestation { attestation_contract_ref }
      | TrustedObserver { implementation_ref },
    domain_evidence_ref,
}
```

Replay reports the assurance actually established by `proof_basis`; it does not collapse a trusted
observation, signed executor attestation, and independently verifiable proof into one claim.

### Required convergence law

The certified executor must guarantee:

- the same effect key and request digest always identify the same logical operation;
- the same key with another request or executor contract is rejected;
- repeated, concurrent, and arbitrarily delayed calls converge on at most one semantic external
  effect;
- any nonce, UTXO, signer payload, external id, or transport choice is durably bound before the
  executor crosses its target mutation boundary;
- replacement, if allowed, remains within the fixed certified semantic request;
- terminal evidence remains retrievable after worker and executor restart;
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

The portable guarantee is:

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
executor binding committed
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

Its durable entry binds effect key, request digest, executor contract, convergence-contract
reference, chosen operation family, and an append-only delivery/evidence log before mutation.
Concurrent calls reuse that binding. Ambiguity never authorizes another operation. Redelivery from
an ambiguous bound entry is legal only under the recorded downstream convergence proof. Terminal
evidence persists before return unless the destination permanently supplies equivalent authority.
If MFM rejects insufficient evidence, later `ensure` calls may return stronger evidence for the
same fixed terminal operation and outcome; they may not rewrite the operation or contradict earlier
evidence.

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

### Authority-bearing execution types

Keep only the public or crate-visible phase types that prevent an unsafe crossing:

```text
StateFrame<S>
  verified typed view over one deterministic input-manifest candidate; no append authority

CommittedRequest<K, T>
  the exact read or effect request with durable authorization/transition authority

AuthorizedAccess<K, T>
  non-cloneable affine authority for one boundary invocation

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

Operations and the certified spec own only static topology, exact typed bindings, dependency/skip
rules, state execution contracts, capability/executor identities, public-output bindings, and
terminal conditions.

Runtime owns verified-view loading, deterministic readiness, exact input materialization, typed
callback dispatch, audited capability orchestration, canonical result validation, transition
construction, and exact-head retry. It owns no domain policy, protocol phase progression, business
retry count, compensation, finality policy, or public projection model.

A read capability owns one external boundary invocation for the exact typed request, including
bounded protocol encoding/decoding, source validation, and safe error classification. It cannot
reduce a state, construct output/facts/failure, choose graph behavior, retry invisibly, or define
replay behavior. A private live-crate helper may translate protocol types, but adapter is no longer
a runtime abstraction or semantic execution layer.

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
async fn drive_once(run_id: RunId) -> Result<DriveOutcome>

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
3. performs at most one semantic transition or one audited capability invocation;
4. appends against the exact journal head; and
5. returns without retaining semantic process state.

One audited invocation may append its authorization and observation as two distinct journal
commits. The “one action” bound is one new live invocation, not one physical store append.

After committing an observation, `drive_once` purely scans it with the retained compatible
observations. It returns `Advanced` only when a settlement candidate now exists. `Returned(Pending)`
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

`CallRead` and `CallEnsure` include authorization, at most one boundary invocation, and mandatory
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
    capability_or_executor_contract_ref,
    admitted_implementation_ref,
    operation_id,
    executor_namespace_ref?,
  )
    -> pure schema/classifier verifiers plus optional live implementation
```

The immutable per-run implementation manifests select only the exact catalog entries required by
the certified spec. A process catalog may be a superset; unrelated additions or removal of
unselected entries cannot change the run contract. Admission and resume fail closed when a
selected entry is missing or mismatched. Catalog construction and binding perform no semantic IO;
provider, source, chain, signer, or route probing belongs in an audited post-admission read state.
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

## Framework-Owned Pre/Post Execution

MFM distinguishes three mechanisms that must not be conflated:

1. **Kernel structural invariants** are unconditional sealed certification/runtime/store checks.
2. **Semantic safety policy** is typed state-machine behavior in the certified graph or in the
   sealed same-transition execution envelope.
3. **Operational telemetry** is derived observation that cannot affect semantic execution.

There is no generic pre-hook, post-hook, middleware callback, or runtime plugin chain.
“Per state” means one certified authored node occurrence. It never means every callback
recomputation, read retry, `ensure` invocation, or worker attempt.

### Same-transition framework envelope

If a pre/post check must pass before the user transition becomes authoritative, it runs as a
sealed typed in-memory phase:

```text
StateFrame<User>
  -> FrameworkPrechecked<User>
  -> UserEvaluated<User>
  -> FrameworkPostchecked<User>
  -> CommitReady<User>
  -> StateTransitionCommitted
```

Only `CommitReady<User>` can construct sealed `CommitTransition` authority. These are not
additional persisted node phases. They are appropriate for universal mechanical enforcement such
as typed input completeness, output schema/canonicalization, no-secret checks, evidence scoping,
and transition-contract validation. Hashing, exact-head CAS, affine authorization,
committed-observation-only reduction, effect-key derivation, object admission, and closure legality
likewise remain unconditional kernel/store code. A skippable or fallible graph node can never own
an invariant required on every path.

Same-transition checks are a sealed kernel set, not third-party callbacks. Their contract/version
and canonical order are bound by the certified state contract and rerun during replay. There is no
framework-function catalog or mutable runtime hook lookup.

### Deferred semantic graph-expansion activation contract

This baseline does not implement semantic graph expansion, persist `NodeOrigin`, install a graph
expander, or inject framework nodes. No concrete separately traceable mandatory semantic rule has
been identified, so shipping generic machinery now would work against the simplification goal.
The following contract records the one accepted direction for a future activation RFC; that RFC
must name and type-check the first real rule and cut its schema, code, tests, and documentation over
together.

When activated, separately traceable pre/post semantics use deterministic framework graph
expansion:

```text
operation builds authored typed program
  -> mandatory FrameworkGraphExpansion
  -> complete expanded typed graph
  -> certification
  -> CertifiedTypedSpec
  -> thin runtime executes only ordinary certified nodes
```

The operation builder is the authoring entry point, but operation code does not choose whether a
mandatory framework rule applies. Every published framework entry point pins and documents a
minimum `CertificationProfile`. The builder applies it while constructing effective handles, and
certification independently rederives and validates the expansion from the retained canonical
authored graph. Only the final expanded graph is runtime authority.

```text
CertificationProfile {
    profile_ref,
    graph_expander_contract_ref,
    ordered_mandatory_rule_refs,
}

NodeOrigin =
    Authored {
        operation_contract_ref,
        operation_call_path,
    }
  | Framework {
        profile_ref,
        rule_ref,
        protected_node_occurrence_id,
        position: Before | After,
        wrapper_ordinal,
    }
```

Authored graph, profile, expander implementation/version, canonical rule order, expanded
nodes/edges, origins, state/capability contracts, and protected-node relations are hash-defining
spec and certificate material. Logical authored occurrence identities freeze before expansion;
final expanded node identities freeze afterward. Policy is never loaded from mutable runtime
configuration during admission, drive, resume, or replay.

Multiple rules use canonical onion order:

```text
Rule1.Pre -> Rule2.Pre -> AuthoredState -> Rule2.Post -> Rule1.Post
```

The order comes from `ordered_mandatory_rule_refs`, never registry discovery or map iteration.

Expansion applies exactly once to authored nodes. Framework-created nodes are not recursively
expanded, child-operation composition cannot expand the same occurrence twice, and user operations
cannot construct `NodeOrigin::Framework`, request profile `none`, obtain raw bypass handles, or
omit mandatory rules. Stable wrapper identities derive canonically from the protected logical
occurrence, profile/rule, before/after role, ordinal, and state contract.

```text
expanded_node_id = H(
    "mfm.framework-node.v1",
    logical_protected_occurrence_id,
    profile_ref,
    rule_ref,
    position,
    wrapper_ordinal,
    state_contract_ref,
)
```

A semantic pre-state is an ordinary certified pure or audited read state:

```text
typed inputs
  -> FrameworkPre
  -> producer-bound SafetyPermit<Rule, ProtectedState>
  -> protected state
```

The activation adds a separate typed `framework_preconditions` section to the protected
`StateFrame` and input manifest, so a certification profile does not change the state's domain
`Input` type. Only the exact framework node occurrence may produce it. Runtime cannot materialize
the protected frame until every required permit verifies against its rule, producer, protected
occurrence, and profile. Guard failure is an ordinary typed failed transition; the protected state
receives a deterministic `DependencySkipped` transition. An external precheck records only what it
observed. If safety can change between check and mutation, the destination or effect executor must
enforce the precondition atomically.

A semantic post-state consumes the protected typed output:

```text
protected state
  -> FrameworkPost
  -> verified effective output
  -> downstream consumers and public output
```

The builder exposes only the effective post-state handle. Certification rejects a public output,
consumer edge, or successful closure path that bypasses a required post-state. A post-state gates
downstream use, publication, and successful closure; it cannot retroactively prevent or validate an
already applied external mutation. Evidence acceptance required for `EffectSettled` remains in the
effect state's pure `settle` callback.

The gate also crosses run boundaries. An ordinary cross-run source names the logical protected
occurrence and resolves under the source certified spec to the effective post-state output. A raw
protected transition remains inspectable history but cannot satisfy an ordinary typed input, fact,
public output, or corrective-run approved-result slot. A corrective workflow that genuinely needs
the raw record must declare a distinct certified evidence-only role as defined below.

Fact emissions require the same precision. A fact emitted by the protected transition is
authoritative and queryable immediately; rewiring a later output edge cannot hide or retract it.
Certification therefore rejects a separate framework post-gate around any fact-emitting protected
state. Fact correctness must either be validated in the same-transition envelope before the
emitting commit, or the protected state must emit zero facts and produce private typed
`FactCandidates<F>`. Those candidates thread through every mandatory post in canonical onion
order; the protected state and every inner/intermediate post emit zero facts, and only the final
outermost effective post may emit them after all gates succeed. Certification rejects a rule
composition that cannot preserve this typed chain or lets an earlier post emit. There is no
unpublished, promote, or retract fact lifecycle.

Framework states receive only certified typed inputs, config, context, and their normal committed
read observation where applicable. They cannot inspect arbitrary journal records, projections,
runtime internals, or raw provider material.

For an effect state, the enforceable order is:

```text
FrameworkPre settles
  -> EffectRequested consumes the exact permit
  -> audited ensure authorization and invocation
  -> committed observation
  -> effect settle verifies terminal evidence
  -> FrameworkPost becomes ready
```

A framework precheck is therefore not a substitute for an atomic external conditional write, and
a post-state is not the terminal-evidence verifier.

An ordinary downstream post-state runs only when its required typed input was produced. It is not a
generic `finally` hook after success, typed failure, and skip. Telemetry covering every terminal
outcome derives from journal records. If a concrete semantic policy must execute after any outcome,
the graph would need an explicit certified settlement-dependency value carrying a closed outcome
and transition reference; that is a new core contract and is outside this baseline.

Injected framework nodes obey the same pure/read protocols, transition trace, audit, replay, and
failure/skip rules as authored states. This RFC forbids framework-injected effect states: a
platform-mandated external mutation is explicit domain workflow or corrective-run behavior, not a
hidden wrapper. A future relaxation requires a separate design.

The framework can guarantee that the protected occurrence never becomes ready without its required
pre permits, that no successful protected output becomes usable or publishable before its required
post-state succeeds, and that every wrapper occurrence is terminal—settled or explicitly
skipped—before any run closes. It does not claim that an ordinary post-state runs after protected
failure/skip, nor can it guarantee finite-time physical execution after permanent worker or
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

- certified spec, certificate, config, seeds, and contexts;
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
    producer_scope: OtherRuns,
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

1. scans the authoritative dense global prefix through the authorization frontier;
2. locates fact-bearing transition records through the validated `emits_facts` routing column;
3. rehydrates every candidate from the journal and object annex;
4. verifies its transition, descriptor, response, and routing evidence; and
5. produces the complete deterministic response.

The state owns query construction and response interpretation. The capability cannot construct
state output, facts, typed failure, graph behavior, or transition authority.

Replay reauthors the same request from the retained base-input manifest, verifies the authorization
and observation, and verifies the response against authoritative history through the recorded
frontier before invoking the same state reducer. This proves omissions as well as selected-item
validity.

A self-contained portable replay bundle must include either:

- the complete contiguous global commit/record prefix from store order one through the pinned
  frontier, verifying every order and every record's fact-routing value; or
- a separately authenticated completeness export/proof containing a frontier census under the same
  query and selection contract.

A collection merely labeled “all fact-bearing transitions” does not prove that a whole qualifying
commit was not omitted. A bundle containing only selected facts or an unauthenticated subset can
verify those items but must report selection completeness as unverified.

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
- `RunObservationReader` scans commit envelopes for list/watch.
- replay invokes the same pure request/evidence/verifier contracts used live.

These are algorithms or opaque proof objects. They never persist a competing lifecycle snapshot.

A fold checkpoint may be introduced only after measurement. It must bind an exact journal-head
digest and fold-version identity and be discarded on any mismatch.

## Replay Contract

Replay performs zero live semantic-capability, provider, executor, network, filesystem-domain, or
signer IO. It may read only the explicitly supplied journal, object annex, fact-history/proof
bundle, and cross-run source dependency bundles through replay storage readers.

Before replaying transitions, it revalidates `RunAdmitted` and every `CrossRunSourceRef` through the
same predicate used by live admission. For each source, the portable dependency bundle contains the
source admission, certified spec/certificate and—if graph expansion is activated for that source—
profile plus authored/expanded graph proof, journal chain through the relevant effective or raw
transition and source closure, referenced output/evidence objects, and certified destination role.
Replay re-resolves effective post output or evidence-only legality; a missing or downgraded source
proof rejects verified replay rather than trusting the destination root hash.

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
  fact-selection response additionally verifies the complete journal prefix or authenticated
  completeness proof through its authorization-order frontier.
- `EffectRequested` reauthors the semantic request and kernel effect key without live mutation.
- `EffectSettled` reuses the request transition's exact manifest, verifies the terminal evidence
  envelope under its stated proof basis, and runs the pure settlement verifier.
- `DependencySkipped` verifies every unavailable source and the certified skip rule without
  invoking the skipped state.

For access audit it:

1. verifies every authorization against its semantic anchor, capability contract, operation, and
   typed request;
2. verifies at most one observation per authorization;
3. permits unmatched and indeterminate authorizations as honest audit history;
4. verifies safe-failure and result schemas;
5. permits a semantic transition to consume only explicitly referenced committed observations;
6. ignores audit records when deriving state unless a semantic transition references them; and
7. accepts post-closure observations only for pre-closure unmatched authorizations.

Replay proves that the recorded typed result follows from certified inputs and accepted evidence.
It does not prove the exact number of physical remote calls or current external truth.

Same-transition framework envelopes rerun as part of transition verification. If a future source
spec activates certified graph expansion, replay independently verifies its profile and canonical
expansion, and injected states replay as ordinary states through the same `StateCatalog`. Replay
emits no operational telemetry and constructs no runtime hook or live capability registry.

## Explicit Corrective Runs

Generic saga policy, remediation roles, obligations, reverse ordering, manual terminalization, and
public `compensated` run modes are removed.

A corrective run consumes the root `CrossRunSourceRef` contract. `EffectiveOutputSource` resolves
the logical source through its certified framework expansion; for an unwrapped source, the source
transition is already effective. `EvidenceOnlySource` is admitted only into an explicitly declared
correction-evidence input slot. It cannot satisfy an approved domain value, ordinary output, fact,
public output, or equivalence claim merely because the raw transition succeeded.

Initially the source run must be semantically closed. Certification and admission verify the source
spec, closure, lineage, role, and effective-output resolution. The new run has its own certified
spec, invocation identity, exact inputs, effect keys, external-access audit, and output.

There is no hidden liveness guarantee between source closure and corrective-run admission. If
guaranteed initiation is required, a domain recovery controller or outbox scans source transitions
and admits corrections with a deterministic source-derived invocation key.

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
physical access trace unless the executor returns linked reviewed audit evidence.

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
contains only the reviewed public semantic request and an optional closed source alias.
Provider-generated request IDs are omitted unless a capability-specific review proves them bounded,
non-secret, non-bearer, and necessary.

`safe_failure` contains only:

```text
stable_code
failure_class
boundary_stage
optional reviewed redacted diagnostic_ref
```

These are closed capability-specific values. The audit API accepts no arbitrary string, metadata
map, provider error object, request body, response body, `Display`/`Debug` output, or error source
chain. If a failure cannot be classified safely, it records only `UnclassifiedFailure`.

Operational timestamps may live in commit envelopes but are excluded from semantic hashes unless a
separate reviewed contract requires them. Worker identity, hostname, PID, lease token, and process
attempt count are not semantic fields.

Audit order, frequency, source alias, and wall-clock time remain privacy-sensitive even when they
are not secrets. Normal run status and transition output do not expose audit internals. A
privileged versioned audit export defaults to omitting source alias and precise time and remains
subject to the unresolved retention policy in `Material Uncertainties`.

Hashed structures remain canonical, domain-separated, and float-free. Bounded-input validation
applies before hashing or retaining provider-controlled content.

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
- a store-wide unique admission logical key for `(operation_id, invocation_identity)`, plus closed
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
fails before publication. This density is part of any portable global-prefix completeness check.

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

Transition inspection exposes:

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

The semantic run export is fixed at its semantic head or closure coordinate. The privileged audit
export is explicitly “complete as of journal head H”; while unmatched authorizations exist it must
not claim that no late observation can arrive.

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
- audited capability contract identity;
- pure effect-request authorship;
- keyed executor identity and convergence assurance;
- explicit executor/domain cross-effect ownership;
- pure terminal evidence verification;
- sealed same-transition framework enforcement contracts; and
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
  implementation manifests;
- complete transition-frame construction and verification;
- audited capability wrappers;
- common bound affine access and committed-observation authorities;
- one read request and one accepted observation per read occurrence;
- the non-overridable same-journal fact-selection binding and its pure recorded-response verifier;
- generic typed read-response materialization of content-addressed value references;
- committed-observation-only reduction;
- keyed `ensure`;
- the exact same state callbacks for live and replay;
- same-transition sealed framework enforcement; and
- transition/audit trace readers.

### App and binaries

Delete generic saga, manual-resolution, worker-attempt, resource-lane, side-effect-phase,
runner-factory, and adapter-lifecycle DTOs, commands, modes, and routes. App assembly supplies the
compiled `StateCatalog` and `CapabilityCatalog` but owns no execution callbacks or framework-hook
registry and cannot replace the reserved fact-selection binding.

Add transition inspection, safe access-audit inspection, pending-effect status, and the one
`drive_once` action plus an optional mechanical drive-until-waiting convenience.

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
- EVM transaction documentation.

Delete `docs/saga.md` after its honest guarantee losses are incorporated into the new design
contract. Explicitly delete the `CertifiedFrameworkLifecycle`, framework public-output runner,
erased runner plan/factory, and Adapter-responsibility taxonomy; do not rename them into the new
model.

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
| Executor binding commits but acknowledgement is lost | Executor reloads the key; it must not choose another operation. |
| Target mutated, executor crashes before terminal commit | The binding remains ambiguous. Redelivery is legal only under the recorded downstream convergence proof; otherwise the executor remains pending and does not qualify for recoverable liveness. |
| Target response is lost | Executor observes or convergence-safely redelivers the bound operation; it never chooses another semantic operation. |
| Executor returns `Pending` | Record the access observation; semantic state remains pending. |
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
- Verify seed, config, context, cell, fact, and cross-run lineage.
- Reject a cross-run `TransitionOutput` or `TransitionFact` in a later input manifest; admit
  cross-run values only through the root `CrossRunSourceManifest` and ordinary cross-run facts only
  through the reserved audited read.
- Reject a `CrossRunSourceRef` from another store scope or epoch.
- Verify before-state digest against the predecessor fold.
- Verify output/fact/evidence binding and after-state digest.
- Verify `RunAdmitted` root lineage and canonical genesis/initial-state test vectors.
- Prove admission atomically binds every selected implementation manifest, cross-run source
  manifest, and complete source-proof object.
- Retry and attach the same logical admission across ambiguous `COMMIT`; reject changed root
  material and prove no second effect-key namespace appears.
- Reject every illegal transition-body field combination and verify dependency-skip blockers/rule.
- Enforce the certified node-class/body matrix and delay closure until every required
  `DependencySkipped` transition commits.
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

- Compile-fail live capability access without its bound `AuthorizedReadAccess` or
  `AuthorizedEnsureAccess`.
- Compile-fail direct calls that try to bypass the audited wrapper or substitute raw request data
  for `CommittedRequest`.
- Prove only `NewlyAppended` authorization mints affine authority.
- Prove idempotent, ambiguous, stale, and rejected authorization results mint no authority.
- Reject authority substitution across run, node, capability, operation, request, and effect.
- Compile-fail or reject direct/forged observation construction without a sealed wrapper result.
- Prove the audited wrapper consumes authority even for `DidNotEnter` and allows at most one
  capability-boundary invocation.
- Reject hidden transport retry in conformance fixtures.
- Record `Returned`, `DidNotEnter`, `Indeterminate`, and unmatched authorization.
- Record unrepresentable response failures without retaining raw unsafe bytes.
- Permit only one observation per authorization.
- Require a committed observation before state reduction.
- Reject an observation and its consuming transition in the same batch.
- Reject stale authorization racing read settlement, effect settlement, or closure.
- Prove adaptive read dependencies require another semantic state.
- Prove unused and in-flight authorizations do not block semantic closure.
- Admit one late observation for a pre-closure authorization and reject every other post-closure
  append.
- Prove audit records do not change semantic state or authorize effect settlement.
- Verify read retries create distinct authorizations and identify the exact observation consumed
  by the winning transition.

### Framework envelope and telemetry

- Prove same-transition framework pre/post checks must mint `CommitReady` before append and that
  any check failure admits no transition or object authority.
- Prove operational telemetry cannot change scheduling, transition payloads, state digests,
  closure, or replay; derive committed telemetry from journal records.

### Future framework graph-expansion activation

These are gates for a follow-up RFC after it names the first concrete mandatory semantic rule; they
are not implementation work in this baseline:

- Golden-test canonical expansion, wrapper IDs, node origins, and onion ordering for direct,
  nested-child, fan-out, and fan-in operations.
- Prove expansion applies once to authored nodes and never recursively to framework nodes or twice
  through child-operation composition.
- Reject an operation or persisted spec that omits, forges, duplicates, reorders, weakens, or
  bypasses a mandatory certification-profile rule.
- Prove only the exact pre-state occurrence can produce the protected state's required typed
  permit.
- Prove a required post-state hides the raw protected output from consumers and public output, and
  that successful closure depends on the effective post-state output.
- Reject cross-run and corrective-run use of a raw protected output as an approved value; resolve
  ordinary source references through the source certified spec to the effective post output.
- Reject a separate framework post-gate around a fact-emitting protected state; prove
  same-transition fact validation or zero protected/intermediate facts plus typed candidate
  threading to the final outermost post. Fail that outer post and prove an inner post emitted
  nothing.
- Prove injected states use only ordinary pure/read protocols, and any injected read uses mandatory
  authorization/observation and the normal replay path.
- Reject framework-injected effects, mutable drive-time policy, arbitrary journal introspection,
  raw output bypass, runtime callbacks, and recursive instrumentation.
- Prove a post-state cannot authorize or reinterpret an already committed effect settlement.

### Keyed effects

- Exactly one pending request per run/node execution.
- Same effect key with another request digest fails closed.
- `ensure` cannot be called before the pending transition commits.
- Repeated, concurrent, delayed, and post-settlement calls converge.
- Inject `target mutated -> executor crash before terminal commit` and require the certified
  downstream convergence behavior.
- Timeout, lookup absence, and provider errors never terminalize the effect.
- Terminal evidence binds key, request, executor contract, external identity, provenance, and
  assurance/finality policy.
- Evidence, output/failure, fact emissions, settlement, and closure are atomic.
- Executor resource ownership survives worker/executor restart, rollback, failover, and split-brain
  attempts.
- Race distinct effects for one executor-owned nonce, UTXO, sequence, or business resource.
- Reject `RunClosed` while any effect remains pending or applied-but-unsettled.

### Replay and views

- Replay performs zero live semantic-capability, provider, executor, signer, or domain IO and reads
  only its supplied journal/object/fact-history and cross-run source dependency bundles.
- Replay recomputes requests and pure reducers from retained inputs/evidence.
- Replay each transition variant and distinguish proof, executor-attestation, and trusted-observer
  assurance.
- Revalidate every effective-output/evidence-only cross-run source from its complete source
  admission/spec/journal/closure/object/role proof plus profile/graph proof when activated; reject a
  missing or downgraded dependency bundle.
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
- Reject portable replay completeness when the contiguous global history prefix or authenticated
  frontier census/completeness proof is absent.
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
- Verify public status and access audit expose only reviewed redacted fields.

### Correction

- Consume a closed source's spec-resolved effective output in a separately certified corrective
  run.
- Admit a raw protected transition only through an explicit evidence-only input role and prove it
  cannot satisfy an approved value, fact, public output, or equivalence claim.
- Reject missing, unverified, nonterminal, or wrong-source evidence.
- Prove source closure creates no hidden correction obligation.
- Deduplicate controller-launched correction by deterministic source identity.
- Require typed equivalence evidence before publishing `compensated`.

### EVM qualification

- Keep transaction mutation unregistered before a keyed executor qualifies.
- Test signer-account coordination across every process/deployment/actor and
  same-key/different-request rejection.
- Test restart, response loss, already-known transactions, rebroadcast, replacement policy,
  success, revert, finality, and reorganization.
- Audit MFM-to-executor accesses without claiming executor-internal RPC coverage.
- Prove raw signed bytes remain outside MFM persisted and public surfaces.

### Store parity and schema

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
- Reject legacy event/projection/saga schemas explicitly.
- Prove old compatibility paths and dual writers do not exist.

## Ordered Logical Commits

1. **`use one committed journal across store runtime and replay`**

   Replace copied committed-stream, projection, artifact, runtime-history, and replay-map authority
   with one native committed journal, one private fold, and one opaque verified view. Preserve the
   current persisted behavior while deleting superseded in-memory representations.

2. **`replace run lifecycle with complete audited transitions`**

   Perform one inseparable vertical cutover across program, spec, certification, events, store,
   runtime, replay, app, binaries, Postgres, tests, and documentation. Introduce complete transition
   records, external-access authorization/observation, affine access authority, keyed effect
   requests, the closed three-case `StateExecution`, `drive_once`, state/capability catalogs,
   sealed same-transition framework enforcement, semantic closure with audit tails, and transition
   facts. Delete worker attempts, custom runners/adapters, replay brokers, phase ledgers,
   saga/manual resolution, resource lanes, universal projections, synthetic completion/retention,
   physical fact-projection authority, old events, and every compatibility path. Cut fact storage,
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
observation. A capability may define a genuine reusable bounded batch; response-dependent or
independent requests are explicit state chains.

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

Rejected. It makes physical access count and ambiguity invisible. Every in-process capability
boundary invocation needs its own authorization and bound affine authority.

### Implement framework safety as runtime pre/post hooks

Rejected. Hooks are absent from the certified graph and journal, can vary across live/resume/replay,
and create another extensible execution lifecycle. Universal structural checks remain sealed
runtime/store invariants. Separately traceable semantic policy is deterministic certified graph
expansion over ordinary states only after the deferred activation contract is satisfied.

### Let each operation opt into mandatory framework wrappers

Rejected as the source of a platform guarantee. Operations are an authoring surface and may use
typed wrapper combinators. If graph expansion is later activated, production entry points pin a
certification profile and the framework expander plus certifier enforce it. An operation cannot
omit or bypass a mandatory rule.

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

### Keep generic saga under weaker names

Rejected. Obligation derivation, remediation ordering, manual terminalization, and compensation
claims remain domain semantics. Explicit corrective runs are smaller and make no automatic
equivalence claim.

## Decision Summary

The target core is an auditable, event-sourced typed state machine:

- one stateless `drive_once` interpreter over closed pure, read, and effect contracts;
- only `Unstarted`, `AwaitingEffect`, and `Terminal` semantic node phases;
- one complete state-transition record per semantic change;
- one atomic commit envelope around each transition and its objects;
- exact input/output/fact/evidence lineage with no generic evidence bag, retained indefinitely at
  first;
- one durable external-access authorization before every MFM-controlled live capability call;
- one mandatory immutable observation for every surviving wrapper result, recording returned,
  pre-entry, indeterminate, or unrepresentable outcomes;
- no claim that a local audit record proves exact physical remote delivery;
- one stable pending effect request and one keyed-convergent executor contract backed by a reviewed
  downstream convergence mechanism;
- verifier-backed terminal settlement;
- one typed read request and one state-accepted committed observation per read occurrence;
- one compiled state catalog, one live capability catalog, exact per-run selected-entry manifests,
  and the same state callbacks in replay;
- cross-run fact selection as an ordinary audited read through one non-overridable same-journal
  capability, with no pre-state query materializer or fact projection authority;
- unconditional framework safety checks in the sealed same-transition envelope, with separately
  traceable semantic graph expansion deferred until a concrete mandatory rule exists;
- spec-resolved cross-run source roles that cannot bypass a future effective post output;
- one fenced writable journal lineage for each store identity;
- operational telemetry derived without semantic authority;
- semantic closure that still accepts constrained late audit observations;
- one private fold and one opaque verified run view;
- facts, status, recovery, replay, public output, and analysis derived from the journal;
- no generic worker attempts, custom runner/adapter lifecycle, replay broker, phase ledger, saga,
  manual resolution, resource lanes, runtime hook chain, or universal projection; and
- explicit typed corrective runs for domain remediation.

If data affects a semantic transition, it must appear in that transition's certified inputs,
accepted observations, outputs, facts, evidence, or before/after anchors. If MFM authorizes a live
external access, that authorization must be durably visible before the call. Anything else is
operational telemetry and cannot authorize runtime state.
