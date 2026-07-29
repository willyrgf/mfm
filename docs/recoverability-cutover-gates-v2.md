# Recoverability Cutover Gate and Inventory v2

Status: contract-closure and schema-freeze evidence for
[`RFC_REFACTOR_RECOVERABILITY.md`](../RFC_REFACTOR_RECOVERABILITY.md)

Contract id: `mfm.recoverability-cutover-gates.v2`

Companion app and transport contract:
[`recoverability-app-surface-v2.md`](recoverability-app-surface-v2.md)

This document records the repository inventory and fixed disposition that closed before the
recoverability schema freeze. The frozen target artifacts are
`contracts/recoverability/v2/annex.json`, `contracts/recoverability/v2/corpus.json`, and
`contracts/recoverability/v2/README.md`; their exact metadata is recorded by the searchable
`C11_ARTIFACT_METADATA` block in the RFC's
[Canonical Schema and Golden-Vector Gate](../RFC_REFACTOR_RECOVERABILITY.md#canonical-schema-and-golden-vector-gate).
This is retained closure and deletion evidence, not a second runtime or persisted-data contract.
[`design.md`](design.md) and [`architecture.md`](architecture.md) are authoritative for the current
implementation.

## Fixed closure decisions

- `mfm-program` owns value-only borrowed `RequestView<T>` and `ObservationView<R>` callback
  inputs. `mfm-runtime` privately owns committed request/observation proofs and binds callback
  results to them.
- `mfm-store` returns a non-cloneable `NewlyAppendedAuthorization` only for a directly observed
  `NewlyAppended` authorization append. Runtime consumes it into `AuthorizedAccess`; ambiguity,
  reload, and `ExistingSame` mint no access authority.
- Every semantic identity hash uses
  `SHA-256(JCS({"domain": exact_versioned_domain, "value": exact_preimage}))`. Prefix/NUL
  framing and compatibility decoding are deleted. Raw retained-byte content addressing remains
  `SHA-256(exact bytes)`.
- The executor's lightweight two-field reference is `mfm_ids::ContentRef`. Journal-retained
  values/evidence use the full producer-bound `mfm_journal::ValueRef` with artifact id, evidence
  hash, schema/semantic type, role, length, media type, and producer binding. A `ContentRef` is
  never journal reachability or access authority.
- `ExecutorDeployment.tenant_scope_id` is the scalar `TenantScopeId`, not a content reference, and
  must equal the admitted run tenant and executor-ledger partition before access.
- Every admitted read binding contains an immutable non-secret `routing_generation_ref` and
  reviewed source scope. A new run may select a new generation; resume resolves exactly the
  admitted generation with no fallback or source reselection.
- `mfm-capabilities` owns one generic `SafeFailure` envelope. Its universal class enum includes
  `cancellation`; class, boundary-stage, and `zero|up_to_16_kib|up_to_1_mib|over_1_mib` size
  vocabularies are closed, while exact EVM, Bitcoin, and executor stable codes/diagnostic unions
  are selected by `safe_failure_contract_ref`. Open diagnostic maps and raw provider text are
  deleted. Each read state contract also fixes the exact callback verdict for every admissible
  code: cancellation, transport, unclassified, and the exact reviewed numeric/busy retry rows
  yield `InsufficientEvidence`; only closed nonretryable numeric destination rejections become a
  typed terminal read-validation failure; and invariant or unrepresentable-response evidence
  yields `InvalidEvidence`.
- All authority-bearing admission, journal, object, and fact-completeness reads, and all writes,
  use the fenced authoritative writer in recoverability v2. Offline stream verification is
  separate. A replica
  cannot claim store-backed authority without a later applied-through-barrier and lineage
  qualification.
- A portable export is the framed `mfm.portable-run-export-stream.v1` JSON text sequence, not a
  DTO or byte wrapper. It contains no digest of itself. SHA-256 over every record separator,
  canonical frame byte, and line feed is returned only as its external `ContentRef` and exactly
  `Mfm-Content-Digest`.
- The generic executor owns its keyed delivery/resource ledger contract; PostgreSQL is one
  possible backend, not the non-rollback or destination-convergence authority. Production requires
  an independently qualified executor/destination fence in addition to the MFM store's separate
  writer/WAL fence.
- The ordinary public surface is exact-run status plus certified public output. Public fact
  browsing and cross-run run list/watch are removed rather than carried through the cutover.
- The core cutover removed the former EVM mutation path. The final sequence step registers one EVM
  wallet effect only through its qualified durable keyed executor, typed sender/nonce owner,
  guarded signer, and independent deployment fence. Bitcoin collection remains unregistered unless
  `scantxoutset "start"` passes its repeat-work-safe read qualification.
- Schema- and implementation-shaping gates, including one retained historical executable
  reproducing in an OS-enforced capability-free boundary, closed before schema freeze. Deployment
  privacy, capacity, authoritative-writer fencing, the long-term executable retention horizon, and
  per-deployment legacy-history disposition remain rollout-only gates.

## Fixed implementation commit sequence

The top-level implementation sequence is exactly:

1. `close recoverability contract and rollout gates`
2. `freeze recoverability schemas identities and golden vectors`
3. `add a durable keyed executor substrate`
4. `use one committed journal across store runtime and replay`
5. `replace run lifecycle with complete audited transitions`
6. `qualify evm writes through a durable keyed executor`

## Repository inventory and disposition

### Generic evidence bags

| Current producer or consumer | Current role | v1 disposition |
| --- | --- | --- |
| `crates/kernel/program/src/lib.rs` — `ExternalReadEvidenceSet` | Couples one primary read value to `Vec<FactQueryEvidence>`. | Delete. A callback receives one exact `ObservationView`; any other evidence is an explicit typed output or fact slot. |
| `crates/kernel/runtime/src/runner_kit/external_read.rs` — `ExternalReadExecution` | Produces the mirrored primary-plus-query-evidence bag. | Delete with the erased external-read runner. |
| `crates/live/portfolio/src/lib.rs` — `SelectHoldingsExecutor` | Only production producer of a non-empty auxiliary evidence list. | Replace same-run selection with graph edges and deliberate cross-run selection with the reserved audited fact capability. |
| `crates/domains/portfolio/src/state/holding_read.rs` | Authors, orders, hashes, and consumes variable-cardinality query evidence. | Replace with typed `FactSelectionRequest`/`FactSelectionResponse` or direct typed inputs. |
| `crates/kernel/runtime/src/runner_kit/output_builder.rs` | Persists query evidence through generic artifact and retention paths. | Delete generic evidence recording and projection-owned retention. Bind exact objects to the transition commit. |
| `crates/kernel/replay/src/v1/value_read.rs`, `crates/kernel/replay/src/v1/broker/facts.rs`, and `crates/app/src/launch_artifacts.rs` | Reconstruct and verify the bag and its cross-run source facts. | Replace with callback-free journal verification plus the exact consumed observation and source-proof closure. |

The Bitcoin and EVM read reducers currently reject a non-empty auxiliary list; they do not justify
retaining the generic type. Store-level required/admitted object bindings are structural atomicity
evidence, not a semantic evidence bag, and become the one `commit_artifact_bindings` relation.

Closure evidence:

- no `ExternalReadEvidenceSet`, `ExternalReadExecution.fact_queries`,
  `record_fact_query_evidence`, or fact-specific loader remains;
- every former producer maps to an exact observation, typed output/fact slot, or deletion; and
- tests reject a generic optional or variable-cardinality evidence-reference escape hatch.

### Pre-admission semantic probes

| Current path | Probe | v1 disposition |
| --- | --- | --- |
| `crates/kernel/runtime/src/runners.rs`, `binding.rs`, and `scheduler.rs` | Generic `validate_ingress` hook at launch and resume. | Delete the hook family. Catalog construction remains pure. |
| `crates/app/src/live_transports.rs` and `crates/live/evm/src/transport/mod.rs` | Loads an EVM route and performs `eth_chainId` before admission. | Bind `routing_generation_ref` at admission without semantic IO; perform source/chain bootstrap as an ordinary audited read afterward. |
| `crates/app/src/live_transports.rs` and `crates/live/bitcoin/src/adapter/mod.rs` | Loads and validates a Bitcoin route/session before admission. | Resolve and validate the exact admitted routing generation in the post-admission Bitcoin bootstrap read. |
| `crates/live/evm/src/adapter/transaction.rs` | Injected mutation route/signer validation. | Removed with the superseded mutation registration; the qualified wallet executor now owns route, signer, resource, and target binding. |

The bounded current-executable read in `crates/app/src/executable_identity.rs`, current
configuration lookup for a new root, and store schema/connectivity checks are permitted platform
bootstrap. They cannot select a live provider result or replace a bootstrap read observation.

Closure evidence:

- no provider, source, chain, signer, route, or executor semantic probe runs before
  `RunAdmitted`;
- resume cannot resolve the same route label to another generation; and
- inability to resolve the admitted generation blocks without fallback or reselection.

### Fact producers and consumers

| Current surface | Current role | v1 disposition |
| --- | --- | --- |
| Bitcoin/EVM read settlement through `RunnerOutputBuilder::record_read_fact` | Emits standalone `FactRecorded` events beside attempt output. | Facts become typed emissions inside the producing transition. |
| `crates/live/portfolio/src/lib.rs` and `crates/domains/portfolio/src/state/holding_read.rs` | Uses `FactQueryStore` as indirect state wiring and may select an equivalent fact from another run. | Same-run data uses graph edges. Explicit prior-run selection is an ordinary audited read through `mfm.journal.fact-selection.v1`. |
| `crates/kernel/store/src/v1/mod.rs` — `FactQueryStore`, memory implementation, and PostgreSQL fact query/projection modules | Provides store-global query and projection authority. | Delete the trait and physical authority. The reserved capability consumes one affine authorization in a private deterministic scan session over contiguous authoritative-writer steps. Only exact-frontier completion constructs a response; no public/persisted continuation, total-prefix validity cap, or alternate fact store remains. |
| `crates/kernel/replay/src/v1` fact broker/evidence paths | Rebuilds fact events and auxiliary query receipts. | Verify transition fact emissions and recorded selection responses from one verified journal view. Portable streams report inclusion only. |
| `crates/app/src/public_facts`, `bin/cli/src/commands/facts.rs`, and `/v1/facts/*` | Public store-wide fact browse/query/reference surface. | Delete without a v1 replacement. Facts remain reachable only through certified state dataflow or separately authorized trace/audit/export closure. |

Fact/closure evidence:

- selected typed values hydrate only through ordinary retained-object/path authority;
- a scan larger than one physical work step reaches the exact authorization frontier without
  truncation or a second authorization, and no caller can supply, serialize, or resume its private
  consuming position;
- portable source verification larger than one source/object/byte step completes from the exact
  DAG root, while missing, cyclic, or tampered material rejects; and
- process loss discards the private consuming session/scratch state, leaves the abandoned
  authorization audit-only, and requires a fresh current-head authorization; neither the abandoned
  attempt nor deterministic restart can mint partial response, completeness, source, or
  verified-view authority.

The baseline also removes public `run list`/watch. A run-bound `ReadPublic` grant authorizes only
that run; operational discovery or a tenant changefeed requires a later, separately scoped
contract.

### Published entry points and planning profiles

The cutover publishes exactly two mappings:

```text
mfm.portfolio/snapshot@1
  -> one canonical authored program
  -> one exact content-addressed PlanningProfile
       framework_policy_refs: []
  -> one planner contract and implementation
  -> one expanded certified spec

mfm.evm/submit-transaction@1
  -> one canonical one-effect authored program
  -> one exact content-addressed PlanningProfile
       framework_policy_refs: []
  -> one planner contract and implementation
  -> one executor-required leaf expansion
  -> one expanded certified spec
```

The empty ordered `framework_policy_refs` list is part of the exact v1 profile bytes because no
production semantic wrapper is presently justified. Synthetic conformance fixtures retain
framework-outer expansion coverage. Adding a future mandatory framework policy creates a
different exact profile and content digest; it cannot silently change this mapping. Executor-inner
expansions remain selected by the exact registered executor contract for each effect and are not
smuggled into the framework-policy list.

Generic Rust authoring helpers may remain authoring APIs, but they grant no admission authority.
Test-support launch helpers, CLI, and REST cannot bypass the registered entry-point/profile
catalog. Closure requires certificate/admission fixtures proving retained authored bytes, the
empty ordered policy list, executor-contract selection, and byte-identical independent expansion.

### Legacy histories, export, and schema rejection

The inventoried pre-cutover store had no portable run export. Its internal
`CommittedRunJournal`/`VerifiedRunView` authority is deliberately non-serializable and does not by
itself carry cross-run source closure, deployment metadata, or historical executable bytes. The
current PostgreSQL v1 authority is concentrated in
`crates/storages/postgres/migrations/0001_store.sql`: commits/events, artifact
blobs/admissions/bindings, fact descriptor/query projections, operational lanes/cursors, and
current configured values.

Before resetting a deployment, its owner must record exactly one disposition:

1. **No retained history required.** Record the inventory scope and owner approval, export any
   current configuration that must survive, then reset with a never-reused store scope and fresh
   epoch.
2. **Retained history required.** Before reset, create and verify an immutable archive containing
   the old database lineage, migration metadata, canonical streams, every referenced/admitted
   artifact and cross-run fact dependency, required configuration, and any externally retained
   historical executable/build environment needed by the chosen reproduction claim.

The new binary explicitly rejects old event, projection, saga, fact-query, certificate/spec, and
store schema families. A one-time exporter must run before cutover against the old schema; it is
not retained as a compatibility reader. Operational projections, lanes, waiters, and cursors are
not semantic archive authority.

### Telemetry and semantic postconditions

Current process telemetry is tracing/logging configured by `crates/app/src/observability.rs` and
emitted by CLI, REST, and transports. It remains best-effort and non-semantic. Health/readiness and
any explicitly retained operational cursor are likewise non-authoritative. No metric, span, log,
cursor, or test synchronization hook may schedule, settle, skip, close, or authorize a run.

Current framework postconditions are implicit public-output rendering, retention, completion, and
saga-terminal nodes injected by `crates/kernel/certify/src/lowering.rs` and exposed through
`CertifiedFrameworkLifecycle`. Delete that machinery. A real postcondition is an ordinary typed
planner-injected state:

- it consumes only declared typed inputs and the protected output;
- every consumer/export is rewired to the effective post output;
- evidence required to settle a read/effect remains in the producing state;
- a post-state cannot inspect arbitrary journal/runtime state or retroactively make a mutation
  safe; and
- failure/skip uses ordinary graph semantics, never `finally`, rollback, unlock, or a runtime hook.

Guaranteed compliance delivery is a visible qualified effect. Best-effort observability remains
derived telemetry.

### Capability qualification disposition

| Capability | Final disposition | Registration evidence |
| --- | --- | --- |
| EVM balance/metadata reads | Registered as source/chain bootstrap, initial anchor, one state per independently meaningful RPC, final anchor confirmation, and pure aggregation. | Exact-call, routing-generation, exhaustive safe-failure verdict, cancellation, partial-failure, anchor, and fan-out conformance passed. |
| Bitcoin balance collection | Aggregate reader removed; collection remains unregistered under its closed disposition. | `scantxoutset "start"` did not pass lost-response, cancellation, concurrent-scan, delayed-reissue, bounded-work/result, and provider-cost qualification. |
| EVM mutation | Registered only as `mfm.evm/submit-transaction@1` through the durable wallet executor. | One shared pre-admission/pre-allocation wallet qualification closes actual route-catalog membership and chain, exact executor semantics/evidence, derived guarded-signer descriptor, nonce policy/configuration, classifier, finality, assurance, tenant/domain/sender/generation/fence, and every evidence bound. After qualification and permanent allocation, one complete wallet-history fold gates target authorization/observation, signer/RPC entry, terminal append, authorization/terminalization-conflict handling, and pending/terminal returns. It folds immutable append order, freezes each authorization's plan from authorization-ordered results observed by then, validates later observations at their physical positions without changing logical result-reference order, selects the first observed valid terminal, and treats later/post-tombstone observations as validation-only. Restored generic-valid hostile descriptors, terminal request/prior-reference/lineage/transaction/receipt/finality/inclusion/outcome/generation/fence/assurance values, outer terminal results, and tombstone relations fail with byte-identical storage and no signer/RPC call; exact restored tombstones and a valid two-inclusion out-of-order/post-tombstone observation tail are read-only. Response-loss/restart, signer-free recovery, already-known, rebroadcast/replacement, success/revert, finality/reorganization, terminal retention, PostgreSQL fencing/refold, and no-secret retention passed. |

## Gate classification

### Closed before schema freeze or relevant capability registration

- Compile-only vertical proof of the program value-view/runtime-proof/store-permit boundary.
- Exact routing-generation schemas and bootstrap read graphs.
- Exact EVM wallet policy qualification before support admission, run admission, effect binding,
  nonce allocation, signing, or RPC, with one shared proof and the `15 + N` live/`68 + N` product
  support closure.
- One EVM wallet-history owner after qualification/permanent allocation for initial, restored,
  post-target, authorization/terminalization-conflict, pending, and terminal decisions, with
  append-order descriptor/result refolding, authorization-time frozen plans, first-observed
  terminal selection, validation-only late observations, complete terminal lineage validation,
  exact tombstone relation, read-only hostile-checkpoint rejection, and signer/RPC/write-free valid
  tombstone replay.
- Evidence-bag and fact-consumer deletions mapped above.
- One exact entry-point/profile expansion, including pure/read/effect, nested-child, fan-out,
  fan-in, framework-outer, and executor-inner shapes.
- Request-author totality for every registered production state, with retained closure evidence in
  [`request-author-totality-audit-v1.md`](request-author-totality-audit-v1.md).
- Passing durable reference executor and two materially different typed resource-policy
  prototypes. The retained managed-PostgreSQL prototype closes the logical contract-shaping gate:
  it covers the closed evidence algebra, atomic effect/resource compare-and-swap, exact-attempt
  destination receipts across delayed, reordered, and exact post-tombstone audit returns,
  cumulative completion reserve, the pre-persistence 256 UTF-8-byte effect-identifier bound
  (1,041-byte ordinal-63 maximum-bound observation and 1,331-byte maximum-bound tombstone under the
  retained accounting, versus a 16,384-byte slot), exclusive and
  finite-inventory/account-sequence policies, strict refolded restore, modeled generation races,
  exact original-binding routing, and hostile bounded decoding. This does not qualify physical
  non-rollback generation handling, WAL/backup lineage, real commit ambiguity, stale/sibling-writer
  exclusion across failure domains, destination-owner coverage, or promotion/failover; those
  remain rollout obligations below. Memory, file, and ordinary unfenced PostgreSQL persistence
  remain insufficient for production registration.
- Tenant fact publication/barrier concurrency, authoritative-writer scans, and memory/PostgreSQL
  parity.
- Generic retained-object fact-response materialization plus deterministic fact-scan and portable
  source-closure continuation evidence in
  [`recoverability-predicate-owners-v2.md`](recoverability-predicate-owners-v2.md#retained-contract-shaping-evidence).
  The gate requires prefixes/closures larger than one work step to complete without total-size
  rejection, and requires discarded scratch to restart without minting partial authority.
- One retained historical executable reproducing in the selected OS-enforced capability-free
  boundary with network and writable host access denied. This closes the implementation-path gate
  before schema freeze without deciding the production retention horizon.
- Canonical schema annex and shared positive/negative golden vectors, frozen in
  `contracts/recoverability/v2/annex.json` and `contracts/recoverability/v2/corpus.json` and indexed
  by `contracts/recoverability/v2/README.md`. Exact artifact hashes and counts are recorded in the
  RFC's `C11_ARTIFACT_METADATA` ledger.
- EVM read and wallet-mutation capability conformance, plus explicit absence of the unqualified
  Bitcoin registration.

### Before resetting a deployment

- Complete its legacy-history/configuration inventory and selected export-or-destroy disposition.
- Verify any required archive before deleting or overwriting the old store.
- Generate a never-reused store scope and fresh epoch.

### Before production rollout, not before schema freeze

- Privacy and retention approval for complete immutable transition/audit/object lineage.
- Capacity approval for initially indefinite retention.
- Availability approval for fail-closed audit authorization.
- Qualification of the authoritative-writer and HA/WAL promotion fence; replicas remain
  non-authoritative in recoverability v2.
- For every registered mutation executor, qualification of its preserved ledger generation,
  stale/sibling-writer fence, destination convergence/resource fence, backup/restore lineage, and
  promotion procedure. Co-location with the MFM PostgreSQL deployment does not merge these
  authorities.
- Historical executable retention horizon and production isolation policy, including when an
  individual run must report reproduction `Unavailable`.

Failure of a rollout-only gate blocks that deployment or capability registration. It does not
create a compatibility mode, optional audit path, replica-authority flag, or alternate schema.

## Material uncertainties

None in the target cutover or gate classification. The listed rollout and capability-qualification
gates remain evidence obligations whose failure blocks that deployment or registration; none
authorizes an alternate owner, total scan/closure validity ceiling, compatibility mode, or partial
authority.
