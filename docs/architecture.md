# Architecture

Status: contributor responsibility and placement guide

`docs/design.md` is the normative contract. This document owns taxonomy, package placement, and
dependency direction.

## Material uncertainties

none

## Runtime shape

```text
operation DSL
  -> pure expansion and certification (mfm-certify)
  -> CertifiedProgramDocument
  -> Runtime -- RuntimeHistoryPort --> private store adapter
       |                                  |
       |                                  +-> sole callback-free fold + backend
       +-> authorized Read  -> registered invoker -> transport/scanner/status port
       +-> authorized Effect -> registered invoker -> transport/signer/resource authority

store purpose readers -> same fold -> purpose-sealed evidence (public/trace/audit/replay/export)
```

Only `State` is executable. `Match` and `FanOut` are structural. Runtime performs one verified
cursor action; it is neither a mutable scheduler nor a workflow-specific lifecycle.

## Package layers

Every workspace package declares one `package.metadata.mfm.layer`.

| Layer | Responsibility |
| --- | --- |
| `kernel` | Domain-free canonical, value, program, certification, history, store, Runtime, and replay contracts. |
| `domain` | Pure domain types, states, structured operations, expansion, and narrow resource ports. |
| `live` | Qualified adapters and reusable protocol transports. |
| `signing` | Generic signing contracts. |
| `secret-provider` | Secret-bearing keystore and signer implementations. |
| `storage` | Concrete RunHistory, configuration-history, activation-registry, or resource-authority persistence. |
| `assembly` | Authentication, qualification, dependency construction, configuration resolution, and application services. |
| `binary` | CLI or HTTP decoding, dispatch, and rendering. |
| `test` | Cross-boundary qualification harnesses and external provider fixtures. |

## Responsibility taxonomy

| Owner | Owns | Must not own |
| --- | --- | --- |
| Canonical/ids | Float-free canonical bytes, domain-separated hashes, typed identifiers, content references | Workflow or domain meaning |
| Values | Strict schemas and producer-independent retained-value contracts | Producer authority or scheduling |
| Program | Typed declaration-ordered authoring, state callback contracts, nominal values and outcomes | Persistence or ambient IO |
| Spec/certify | Serialized structured algebra, pure expansion, bounds, failure plans, policy and implementation closure | Runtime scheduling or live IO |
| Journal | Exactly five strict record families and assigned append identities | Persistence, callbacks, or cursor derivation |
| Store | Atomic CAS, exact object closure, sole callback-free fold, verified cursor, purpose readers, writer fencing | Domain interpretation or destination IO |
| Runtime | Sole history writer, admission, one cursor action, affine access bracket, callback invocation | Business policy, EVM lifecycle, or cross-run resource state |
| Replay | Portable export format, offline verification, and callback-free projections over the store fold | Writer, callback, provider, or signer authority |
| State | Deterministic request authoring and returned/safe-failure interpretation | Network, filesystem, clock, store, or hidden retries |
| Adapter | Total binding from one Runtime authorization to one explicit capability | Scheduler, persisted lifecycle, or unrecorded completion |
| Transport | One bounded protocol exchange and checked public response | Run history, state settlement, or resource policy |
| Resource authority | One narrow durable cross-run invariant and its permanent operation keys | Run folding, scheduling, provider calls, or terminal run meaning |
| App | Qualified assembly, access policy, tenant isolation, config resolution, DTO orchestration | History mutation outside Runtime or domain logic |
| Binary | Input parsing, command/route dispatch, rendering | Lower implementation construction or secret retention |

## Kernel dependency direction

```text
canonical + ids
  -> values
  -> capabilities
  -> program + spec
  -> certify
  -> journal
  -> runtime  (process authority + RuntimeHistoryPort)
  -> store    (private adapter, sole fold, purpose readers)
  -> replay
```

Some lower primitives are siblings rather than a strict linear chain, but dependencies always
point toward contracts with less authority. The kernel never imports a concrete store, domain,
live adapter, app, or binary. Generic Runtime, journal, store, and replay must remain EVM-neutral.

Concrete stores depend on their domain-free store port or domain-owned resource port. Pure domains
may depend on kernel contracts and generic signing descriptions, not live implementations or app.
Live crates consume their domain and reusable transports. Assembly wires everything. Binaries
consume only assembly/public contracts.

## Structured operation placement

`mfm-program` provides typed builders. An operation deterministically:

- declares input roots in order;
- appends states, exhaustive matches, and bounded fan-outs;
- binds child inputs and exact child success/failure boundaries;
- selects explicit recovery or exact default failure mapping; and
- constructs one nominal root outcome.

Operations perform no network, filesystem, signer, store, configuration, or current-run IO.
Domain-specific expansion belongs with the domain and is registered with `mfm-certify`. Expansion
injects ordinary visible states and structure; no Runtime branch depends on expansion origin.

## State and capability placement

A state selects `Pure`, `Read<C>`, or `Effect<C>` and declares exact input, output, failure,
request, returned, safe-failure, fact, and capability contracts. Pure callbacks receive typed
canonical inputs only. Read/Effect bind distinct returned-value and safe-failure settlement
callbacks: returned settlement may propose success, typed failure, or `InvalidEvidence`;
safe-failure settlement is disposition-typed and success-only under `SafeFailureSuccessOnly`, so
every inhabited safe-failure value settles without a reviewed sample corpus. The store append
creates durable authority from accepted proposals.

A capability represents one application-protocol request. A registered process implementation
validates request, returned value, and safe failure. A live adapter additionally owns the private
physical binding and total invoker. Transport errors are mapped to reviewed safe failures or
generic non-state-consumable access outcomes before persistence; provider text is discarded.
Live registration also emits the exact public physical purpose tuple used by production assembly:
capability, adapter implementation, physical target, and its immutable ordered certificate-release
history. This catalog contains evidence only and confers no invocation or mutation authority.

## Store and Runtime placement

`mfm-store` is the choke point for persisted legality. Its backend accepts a store-validated
candidate, not arbitrary records. The in-memory backend is a conformance implementation. The
PostgreSQL backend owns exact-target session capabilities, role-separated pools, per-transaction
target permits, SQL transactions, and fence-generation enforcement but reuses the same fold.
Ordinary assembly receives only opaque deployment-issued session bundles, never a pool, URL, raw
fence, or DML transaction.
The fold marks physical checks as retained-history replay or current-candidate qualification and
supplies the exact prior binding plus folded minimum lineage head for refresh. A deployment
verifier can consequently retain historical releases for callback-free restart while requiring
new Effect attempts and supersession proofs to follow one strict old-to-new release relation.

Prior-run fact selection keeps its pure and persisted responsibilities separate:

- `mfm-facts` owns the journal-independent request, bounds, queries, typed Read values, and fixed
  deterministic selector.
- `mfm-journal` owns the admitted source-manifest contract, tenant publication/barrier coordinates,
  scanner binding certificate, selected-source provenance, and completeness attestation.
- `mfm-store` recognizes the exact reserved Read, assigns publication and barrier coordinates,
  mints its affine purpose-limited permit, scans and verifies producer histories, and recomputes
  retained positive responses during public verified loads.
- storage backends atomically compare both the run predecessor and tenant fact head. The memory
  backend mirrors the production contract; PostgreSQL retains dense heads and append-only
  publication routes in `tenant_fact_heads` and `tenant_fact_publications`.
- `mfm-certify` owns deterministic expansion, predicates, proof construction, and the concrete
  `AdmissionVerificationRegistry`. Process/live invocation authority is held by Runtime after
  store assembly consumes one complete `QualifiedProgramRegistry`. The reserved prior-run fact
  scanner remains a kernel process baseline installed during registry qualification. The source
  retains no backend, pool, writer, or generic query handle. Registry finalization requires the
  complete expected entry-point identity set and rejects missing, extra, or duplicate identities.

Runtime holds a consumer-side `RuntimeHistoryPort` and the process registry. The history port,
physical-binding verifier, wallet authority, and PostgreSQL checkpoint ports all inherit
workspace-private authority markers; ordinary downstream crates therefore cannot implement a
look-alike authority by satisfying the visible methods. Production adapters and backends stay
private to store assembly; Runtime never receives a raw backend or writer. It
loads one verified prefix, selects the minimum actionable occurrence path, and performs exactly one
action. The scanner travels through the same ordinary Read
authorization/invocation/observation/settlement protocol. Runtime passes the newly committed
authorization proof into the sealed invoker, and every protected adapter retains that proof until
its provider call completes. No normal live return can escape recording. The private store adapter retains at most one same-fold verified
successor between drives and validates its journal head against the indexed backend head before
reuse; a stale or absent entry uses the sole full fold. The indexed-head query is the snapshot point
for a non-mutating load, while exact-head compare-and-append protects every mutation from a later
external append.

`mfm-certify` issues opaque process identities containing semantic kind, semantic contract, and
qualified implementation contract. Runtime retains that identity on callback proposals and emits
one contextual fault carrying the exact phase, run, prior verified head, occurrence, and process or
store identity. The app converts it to a reviewed public attribution that omits implementation
contracts and lower-boundary diagnostics. A callback or rejected candidate never becomes a
history record; durable integrity blocking requires an invoker-returned committed integrity
observation.

Replay receives only a reader. It cannot invoke state callbacks, append, refresh physical
bindings, scan arbitrary data, contact providers, or construct signers.

## Configuration placement

Configured values are not run events. The store layer owns an append-only configuration-history
port and PostgreSQL implementation. Deployment/maintenance receives the writer; application
assembly receives a reader. Admission resolves one exact `(store, tenant, entry, target)` revision
and includes that immutable revision in the run material.

The PostgreSQL adapter also owns a per-stream `configuration_heads` exact-head CAS. It detects
local removal, rollback, and divergence, but it cannot independently detect a coordinated rollback
of the database rows and local head. Deployment qualification therefore combines it with the same
external non-rollback writer-fence authority admitted for production storage; neither the local
head nor a reconstructed database may mint that authority.

## EVM placement

`mfm-evm` owns:

- EVM request/result/failure types;
- chain-registry declarations and attestations, full chain-instance bindings, and route-membership
  catalog descriptors;
- structured balance and submission programs;
- registered submission expansion;
- wallet-domain, intent, reservation, candidate, activation, and completion contracts; and
- the narrow wallet activation/nonce authority port.

`mfm-evm-live` owns direct Runtime-authorized JSON-RPC, balance, signer-attestation, exact
broadcast, and wallet-authority bindings. Its signer-attestation constructor accepts only
`mfm-signing`'s opaque process-local `QualifiedReadSigningProvider`, and it rechecks the bearer's
immutable observational eligibility before every signer callback. It owns no scheduler or durable
transaction lifecycle.

`mfm-storage-evm-postgres` owns real SQL activation-registry and nonce-authority implementations,
private role-specific pools, target-session checks, transaction-bound mutation permits, permanent
operation-key idempotency, and linearizable status. It performs no JSON-RPC and cannot append run
history.

Qualified deployment infrastructure owns the non-exportable target key, fence issuer, session
qualification, chain-registry and route-membership issuance, non-rollback registry head,
revocation, promotion, and complete sender-path fencing. Its provider protocol is the sole path
that turns a complete public routing catalog into the opaque non-serializable qualification value
accepted by application assembly. Live bindings, portfolio compilation, wallet activation,
transaction intent, and submission transport all compare the full qualified chain binding;
chain-id equality is never sufficient. Repository assembly consumes the sealed client/enforcement
port and fails closed when it is absent.

Application assembly owns the `BeginDeploymentAssembly`/`FinishDeploymentAssembly` cutover. Begin
consumes the opaque qualified catalog and binds the exact current wallet provenance, key-specific
signer evidence, code-derived semantic tuple, six complete release histories, catalog, and private
route inventory to a bounded provider lease and fresh target challenges. Finish validates the
ordered endpoint proofs, rechecks catalog/fence/wallet currentness, and removes the lease in the
same critical section that authorizes completion. Only then does the app consume the
provider-neutral completed exchange and qualified signer and privately construct all live
bindings. Storage does not depend on the live transport crate, and neither public DTOs nor a
directly constructed transport are deployment authority.

`mfm-signing` owns the reusable raw guarded-provider contract and the sole checked conversion into
`QualifiedReadSigningProvider`. Raw providers and their transitive guards must explicitly reject
Read eligibility when they consume quota, approval, anti-replay, billing, rate-limit, or other
semantic state. `mfm-keystore` proves the actual key identity and guard/fence contract during both
initial qualification and affine handoff through a signer-owned, token-gated open/decrypt path that
cannot initialize or persist keystore state. The old audited getter is deleted; its authenticated
`v1` audit variant remains schema history with no executable producer. The observational provider is implementation `v2`; an existing signer release
history retains `v1` and appends a same-target `v2` successor rather than rewriting the old release.
The application carries the pending keystore authority
through provider assembly, consumes it into the opaque Read-qualified bearer, and maps any handoff
failure to its reviewed redacted deployment error. Neither structural signer descriptors nor
release-history objects can mint or substitute for that live qualification, and there is no Effect
fallback.

The provider's registry issuance reference commits its logical provider identity together with the
activation record. Startup hydration recomputes that commitment, permitting same-identity process
restart but rejecting cross-provider activation reuse. Revocation closes new assembly admission
and clears pending assembly leases atomically; a concurrent Finish succeeds only if it linearizes
before that cutover. Lease observation, revocation drain, and promotion drain account for both
ordinary authority leases and nonexpired assembly leases.

The external checkpoint owner is a separate control-plane authority outside every restartable
provider child. It retains one acknowledged prefix and at most one exact prepared transition.
Children prepare before SQL, acknowledge only the observed exact successor prefix, and reconcile
before readiness: predecessor retains `Prepared` for identical-only retry, successor finalizes it,
and rollback, database-ahead, sibling-successor, or target mismatch rejects startup. No child-local
file, copied database, or public attestation can reset that checkpoint.

## Portfolio placement

`mfm-portfolio` owns configuration validation, route-to-lane compilation, the depth-two structured
snapshot operation, and aggregation semantics. It imports EVM domain operations, not EVM live
adapters. App assembly resolves the configured portfolio and routing manifest before authoring the
candidate; certification admits only a program within the frozen entry-point envelope.

## Application and binary placement

`mfm-app` builds one qualified registry, splits it into the store verifier and Runtime process
registry, opens the fenced store, gives Runtime the writer, and retains only readers plus the
application facade. Access grants are re-evaluated for each admit, drive, read, replay, trace,
audit, and export call. The policy derives tenant and stable principal from the credential;
admission authorization names the exact operation, configured target, and invocation before
configuration resolution. Export authorizes the sealed root and every recursively referenced
prior-run source under the same target, tenant, principal, and export purpose before any byte is
serialized. EVM configured values own semantic transaction material only, and the app constructs
the identity-bound request after authorization from that configuration plus the selector's bounded
caller token. Tenant equality is checked after callback-free load.

CLI and REST preserve their transport contracts. Standalone binaries do not mint deployment
authority. They fail closed until an embedding deployment supplies the fenced store and sealed EVM
bindings.

## Current runtime-history cutover

The store's canonical ingress is shared by memory and PostgreSQL. It validates the complete
append envelope, persisted-object closure, bounded counts, canonical bytes, and exact predecessor
before either backend performs DML. PostgreSQL loads query the indexed head before loading any
batch, object, or revision rows and revalidate target and external-checkpoint lineage before the
snapshot is released. Checkpoint state is keyed by the stable `(store, epoch, target, stream,
stream-id)` identity; a predecessor is a mutation precondition, never a second stream identity.

Runtime access proofs are consumed by both Read and Effect adapters. A qualified adapter must bind
the proof's access kind, state input where applicable, and retained physical certificate before
entering a target. The unqualified Effect invoker is integrity-fault-only. EVM recovery observes
every retained candidate in ordinal order before replacement and stores one compact, canonical
recovery closure for completed wallet state.

Portable export evidence hands the encoder canonical batch frames rather than mutable store batch
objects. Recursive source closure is checked for exact direct dependencies, cycles, and maximum
required producer heads before encoding.

## Contributor checks

Before adding a public type, module, schema, crate, route, or task:

1. Name exactly one owner row above.
2. Confirm the dependency points toward a lower-authority contract.
3. Confirm no second fold, scheduler, mutation writer, configuration authority, or resource
   lifecycle is introduced.
4. Route ambient IO through a registered adapter/transport or narrow authority.
5. Keep persisted structures canonical, float-free, strict, bounded, content-addressed, and
   secret-free.
6. Add boundary tests and update this document when responsibility changes.

The removed arbitrary execution graph and generic executor are not compatibility surfaces. A
static reference-closure graph used by certification, Cargo, or documentation is ordinary
dependency data and never execution authority.
