# MFM Architecture Guide

> Contributor-facing architecture overview.
> Normative design contract: [`docs/design.md`](design.md).

## 1. How To Use This Document

Use this guide to decide where code belongs and what invariants must hold.

Read order for contributors:
1. This document (`docs/architecture.md`) for boundaries and implementation placement.
2. [`docs/design.md`](design.md) for normative execution and storage semantics.
3. [`AGENTS.md`](../AGENTS.md) for workflow, CI parity, and contribution rules.

For the current registered ops and production states, see [`docs/ops-and-states.md`](ops-and-states.md).
For canonical RPC control-plane routing, bootstrap, and executor-boundary behavior, see
[`docs/evm-rpc-routing.md`](evm-rpc-routing.md).

If code conflicts with `docs/design.md`, treat that as a contract violation until the contract is intentionally updated.

## 2. MFM In 5 Minutes

MFM is an event-sourced execution engine for reproducible workflows.

A run is defined by three immutable surfaces:
- A content-addressed manifest.
- An append-only `run:*` stream in the shared stream store.
- Content-addressed artifacts (snapshots, facts, outputs).

Execution unit model:
- Ops expand into state graphs.
- The runtime executes states.
- Ops are planners; states are executors.
- CLI/REST start or resume runs and render outputs.

## 3. Non-Negotiable Invariants

1. Append-only run streams.
- Historical records are never mutated; for `run:*`, those records encode machine events.

2. Per-append atomicity.
- Each `StreamStore::append(...)` is all-or-nothing.
- Each `StreamStore::append_batch(...)` is all-or-nothing across every touched stream.

3. Attempt envelope semantics.
- Each state attempt is bounded by `StateEntered` and exactly one terminal kernel event (`StateCompleted` or `StateFailed`).

4. Checkpoint advancement rule.
- Only `StateCompleted { context_snapshot_id }` advances resume checkpoints.

5. Content addressing everywhere.
- Manifests, snapshots, facts, and outputs are immutable and hash-addressed.

6. Canonical JSON hashing.
- Hashed structured data uses canonical JSON semantics (RFC 8785/JCS-style target).
- Floats in hashed structures are forbidden; use integer-scaled values or decimal strings.

7. No ambient IO in state logic.
- State handlers interact through an IO provider abstraction that supports live and replay.

8. No secrets in persisted or contract surfaces.
- Secrets must never appear in manifests, run-stream records, artifacts, snapshots, CLI/API outputs, or error details.

9. Thin binary boundary.
- Domain execution logic belongs in ops/state-machine layers, not in `bin/cli` or `bin/rest-api`.

10. EVM network transport boundary.
- Runtime EVM network calls must enter through the `rpc.control` transport
  wiring in `crates/app`.
- Canonical managed `rpc.control` requests must carry explicit `network_id`.
- Managed request identity includes `control_scope`; callers may default it to
  `shared`, but the effective value must be stamped into the serialized request
  before hashing or replay lookup.
- Managed bootstrap source catalogs must declare `network_id` for every source.
- `namespace: "evm"` remains an internal/direct executor surface used by
  `rpc.control` and explicit low-level tests; it is not the contributor-facing
  routing authority.
- Local keystore and local signer paths (`local.keystore.*`, `local.evm.*`) remain intentionally
  offline/local and must not be forced to depend on RPC availability.

## 4. Runtime Mental Model

A run lifecycle:
1. Build manifest and execution plan.
2. Emit `RunStarted`.
3. For each state attempt: emit `StateEntered`, run handler, emit terminal event.
4. Persist snapshot references for successful states.
5. Emit `RunCompleted`.

Replay/resume behavior:
- Resume is checkpoint-based using kernel events.
- Replay uses recorded facts/artifacts; missing facts return structured errors.
- Retry behavior is policy-driven by run configuration.

## 5. Composition Model

### Three-tier thin-layer principle
The canonical architecture follows the three-tier thin-layer model below:

```text
bin/{cli,rest-api}                 (THIN) transport parsing/routing/output only
          |
          v
op definition crates               (THIN) config validation + state graph wiring
  crates/ops/*-op
          |
          v
shared state crates                (THICK) reusable single-responsibility states
  crates/states/common/src/states/*
  crates/states/keystore/src/states/*
  crates/states/aave-v3/src/*
  crates/evm-runtime/src/states/*
```

Core rule:
- executable logic lives in reusable states
- ops assemble state graphs
- binaries stay transport-only

Allowed exception:
- op-local state implementations are acceptable only for domain-specific output/aggregation states
  that are not reusable shared primitives.

For the current inventory of registered ops and production states, see
[`docs/ops-and-states.md`](ops-and-states.md).

### Strict op vs state contract
Operation (`impl Operation`) is a planning abstraction:
- MUST validate op input shape and produce deterministic planning output.
- MAY recursively compose sub-operations and/or directly produce state-graph structure.
- MUST converge to a deterministic final `StateGraph` before runtime starts.
- MUST declare op imports/exports (`OpInterface`) as an interface contract.
- MUST NOT execute runtime side effects.
- MUST NOT perform ambient IO (`fs/network/time/env/process`) in `expand()`.
- MUST NOT implement `State` handlers in reusable paths.

State (`impl State`) is an execution abstraction:
- MUST execute exactly one runtime step (`handle`) with explicit context + IO provider usage.
- MUST encapsulate executable workflow behavior (including side effects via `IoProvider` only).
- MUST be reusable where practical; keep scope single-responsibility.
- MUST NOT perform pipeline/topology planning.
- MUST NOT invoke operations or request planner re-entry.
- MUST NOT parse transport payloads or own CLI/REST contract decisions.

Rule of thumb:
- If code builds `StateNode`/`DependencyEdge`, it belongs to an op crate.
- If code implements `State::handle`, it belongs to a shared state crate unless it is a justified op-local output/aggregation state.

### Ops are planning abstractions
- An op resolves to deterministic planning output given `OpConfig` and `RunConfig`.
- That planning may recursively compose sub-ops before flattening to a concrete `StateGraph`.
- `expand()` must be deterministic and must not perform IO.

### Recursive operation planning
- MFM should converge on one planning abstraction: recursive operation planning.
- A root op may expand into sub-ops; sub-ops may expand recursively or directly into states.
- The planner must flatten the result into one final `ExecutionPlan` before runtime starts.
- Runtime executes states only.
- States must never invoke ops or reshape the graph.
- Child ops need deterministic hierarchical identities during planning even if runtime-visible
  `StateId`s stay flat in v1.
- Context namespacing and import/export wiring must use full child-op paths.
- Duplicate child-op paths, duplicate exported ports, and duplicate flattened `StateId`s are plan
  errors.
- The exact operation API may evolve to represent child-op expansion explicitly; the invariant is
  flatten-before-runtime, not a specific helper signature.

### Semantic core and action-layer boundary
- Core execution semantics are: `Subject`, `NetworkView`, `Instrument`, `Position`, `Venue`,
  `Valuation`, and `Observation`.
- `Observation` is the canonical execution read-model; `Snapshot` and `Report` are projection/read-model
  layers produced from observations.
- `ContractRef` and transport locator details remain adapter-owned implementation details.
- `ActionSemantics` is reserved as the action intent vocabulary (for example `Swap`, `Borrow`, `Lend`,
  `Repay`, `Deposit`, `Withdraw`, `Stake`, `Unstake`) and is intentionally separate from current
  observation-first execution semantics.
- `ActionSemantics` integration is future work; the current implementation treats it as a stable type
  namespace while planner/runtime execution focuses on observations and projections.
- `ObservationKey` is planner-owned identity for deterministic ordering and duplicate detection.
- Canonical split remains: ops compile a fixed semantic runtime topology; runtime executes states only.

Practical dispatch split:
- planner-time dispatch owns semantic validation, exact adapter selection, batch partitioning,
  child-op path assignment, import/export validation, and optional compiled-plan artifact emission
  for audit/inspection
- runtime-time dispatch owns exact adapter lookup by planned id plus IO execution
- runtime must not rerun capability matching, rediscover batch topology, or reshape the graph

### Flattened pipelines
- Multiple ops are flattened into one execution plan and one run.
- The same flattening rule should apply whether composition came from caller-supplied pipelines or
  recursive op expansion.
- Shared context is namespaced by full op path; cross-op imports/exports are explicit.
- Recursive child-op composition must use the same collision rules as pipelines: no implicit
  "last writer wins" export behavior.

### Nested child runs
- Supported conceptually, but engine-managed child runs are deferred.
- When introduced, parent/child linkage must be explicit in events.

## 6. Boundary Map (Where Code Belongs)

### `crates/machine/`
Owns runtime correctness:
- state traits and metadata
- execution plan representation
- kernel events
- executor start/resume semantics

Must not own:
- chain/business logic
- concrete storage backends
- transport layer behavior

### `crates/sdk/`
Owns orchestration ergonomics:
- operation registry and planner helpers
- recursive child-op flattening and hierarchical op-path assignment
- context namespace wiring and import/export validation
- optional compiled-plan artifact emission for audit and inspection
- launch/resume glue over machine + stores
- machine-friendly pipeline interfaces

Must stay thin and avoid domain-specific execution behavior.

### Shared state layer crates
`crates/states/common/` owns cross-domain reusable state primitives and shared op-level helpers.
Domain-specific reusable states may live in dedicated shared-state crates
(`crates/states/keystore`, `crates/states/aave-v3`, `crates/evm-runtime`).

See [`docs/ops-and-states.md`](ops-and-states.md) for the current concrete inventory and owning
crates.

### `crates/ops/*-op`
Owns domain workflow planning:
- operation config validation
- graph composition using reusable states
- op-level tests and domain contracts
Prefer thin ops: avoid embedding thick executable logic when a reusable state belongs in a shared-state crate.

See [`docs/ops-and-states.md`](ops-and-states.md) for the current built-in op catalog.

### Breaking Internal Refactors
Internal crate/module path migrations may be performed as hard cutovers:
- no compatibility re-exports
- no deprecated aliases
- all in-repo consumers updated in the same change

### `crates/collectors/*`
Owns external data collection and normalization adapters.

### `crates/transports/*`
Owns local/internal live transport factories that are not external collectors.

### `crates/storages/*`
Owns stream/artifact persistence implementations only.
Correctness-critical control-plane coordination uses a sibling Postgres storage crate
(`crates/storages/control-plane-postgres`) that writes control-plane stream-family records into the
shared append-only stream tables and updates projection tables in the same SQL transaction.
This boundary must not push control-plane semantics into `crates/machine`, and it does not widen
the generic `StreamStore` trait for v1.

### `crates/core/`
Owns primitives and security-sensitive keystore/crypto code.

### `bin/cli/` and `bin/rest-api/`
Own transport adaptation only:
- parse requests
- invoke run start/resume/query APIs
- render stable output contracts

### Live IO adapter model
Canonical layering:
- Layer 1: domain adapter clients (typically `crates/collectors/<domain>/`) expose typed request/response APIs over `IoProvider`.
- Layer 2: live transport factories (`crates/collectors/<domain>-<impl>/` for external sources, `crates/transports/*` for local/internal concerns) perform real IO.

Runtime wiring rules:
- States depend only on `IoProvider` or typed domain adapter clients; states do not depend on transport factories.
- Transport assembly uses `TransportRegistry` (`HashMapTransportRegistry`) + `RouterLiveIoTransportFactory::from_registry(...)`.
- Every transport factory declares a unique `namespace_group()`. Duplicate groups are invalid.
- Namespace routing is hierarchical longest-prefix matching (`group` or `group.*`), so narrow groups override broader groups.
- Deterministic computed outputs should use `IoProvider::record_value(...)` for fact recording instead of passthrough transport no-ops.

### Semantic adapter system (portfolio domain)
The portfolio domain (`crates/states/portfolio/`) uses a semantic adapter architecture:
- `SemanticCatalog` — deterministic adapter registry with exactly-one-match selection for observation, subject, view, and valuation planning.
- `AdapterId` — transparent string newtype using the slash convention `"capability/family/implementation"` (canonical v1 contract). Use `AdapterId::parts()` for structured access.
- `VenueId` — transparent string newtype. Venue metadata lives on the separate `Venue` struct.
- Fixed semantic state family — 8 states (PrepareExecutionSources through ProjectReport) form the runtime topology; variation enters through adapters, not state branching.
- Multi-network support — the same semantic pipeline handles EVM and Bitcoin wallet observations.
  Bitcoin IO uses `crates/collectors/btc-jsonrpc-http` (JSON-RPC over HTTP via `reqwest`), dispatched through the `rpc.control` transport when `MFM_BTC_RPC_URL` is configured.

## 7. Thin-Binary Request Flow

Target flow:
1. Binary parses input and validates transport shape.
2. Binary calls app/sdk launcher for run start or resume.
3. Runtime executes op-derived states.
4. Binary renders stable text/json output from run results.

Anti-patterns:
- signing transactions directly in binaries
- embedding workflow branching in route handlers
- direct persistence/business orchestration in transport commands

## 8. Observability Contract

- Use `tracing` for library and binary instrumentation.
- Logs go to stderr; contract payloads stay on stdout.
- Runtime log env contract:
  - canonical baseline: `LOG_LEVEL`
  - component overrides: `MFM_LOG` (and `RUST_LOG` compatibility fallback)
  - format aliases: `LOG_FORMAT` / `MFM_LOG_FORMAT`
  - span-event aliases: `LOG_SPAN_EVENTS` / `MFM_LOG_SPAN_EVENTS`
- CI/service diagnostics env contract:
  - canonical diagnostics level: `LOG_LEVEL`
  - canonical diagnostics output routing: `OUTPUT_MODE` (`stdout|logs|both`)
  - shell runtime enforces strict `LOG_LEVEL` enum values (`error|warn|info|debug|trace`); use `RUST_LOG`/`MFM_LOG` for target-specific composite filters
  - compatibility aliases are supported: `NIXFIED_LOG_LEVEL` and `NIXFIED_OUTPUT_MODE`
  - project CI setup rejects legacy CI env vars: `CI_LOG_LEVEL`, `CI_VERBOSE`, `NIXFIED_VERBOSE`, `NIXFIED_DEBUG`
  - if `OUTPUT_MODE` is unset and `LOG_LEVEL=debug`, CI defaults to `OUTPUT_MODE=both`
  - CI step command output follows `OUTPUT_MODE`: `logs` captures to artifacts only; `stdout|both` stream while capturing
  - parity/CI teardown captures per-service status/events and debug-level service logs (`LOG_LEVEL=debug|trace`) for orchestrated fixtures, independent of `OUTPUT_MODE`.
- Include correlation fields when available:
  - `request_id`, `run_id`, `op_id`, `state_id`, `attempt`, `artifact_id`, `event_seq`.
- Logging and error rendering must follow no-secrets policy.

## 9. Security Contract (Contributor Summary)

- Keystore and crypto paths are security-sensitive and require stronger test coverage.
- Avoid accidental secret copies; use zeroizing buffers where appropriate.
- Never include secret material in fixtures, snapshots, errors, logs, or docs examples.

## 10. Testing Requirements By Change Type

If you touch runtime/resume semantics:
- Add crash/resume tests.
- Add replay determinism tests.

If you touch storage semantics:
- Add append atomicity/concurrency tests.
- Add artifact integrity/content-address tests.

If you touch keystore/crypto paths:
- Add tamper/corruption tests.
- Add secret-handling regression tests.

If you change CLI/REST behavior:
- Preserve output schema stability.
- Update command/endpoint documentation in the same change.

## 11. Contributor Placement Checklist

Before adding code, decide scope:
- Does it implement `Operation::expand` and build graph topology? Put it in `crates/ops/*-op`.
- Does it implement `State::handle` and execute runtime behavior? Put it in a shared state crate (`crates/states/common`, `crates/states/keystore`, `crates/states/aave-v3`, `crates/evm-runtime`) unless it is a justified op-local output/aggregation state.
- Is it runtime semantics? Put it in `crates/machine/`.
- Is it transport parsing/rendering only? Put it in `bin/cli` or `bin/rest-api`.

If the answer spans multiple layers, split responsibilities explicitly rather than letting binaries absorb domain logic.

## 12. Related Documents

- Current ops/states inventory: [`docs/ops-and-states.md`](ops-and-states.md)
- Normative contract: [`docs/design.md`](design.md)
- Semantic execution and recursive planning principles: [`docs/RFC_SCATTERED_STATES_TO_SEMANTICS.md`](RFC_SCATTERED_STATES_TO_SEMANTICS.md)
- Offline cache/vendoring planning notes: [`docs/RFC_VENDORED_PROJECT.md`](RFC_VENDORED_PROJECT.md)
- RPC control-plane and EVM executor runbook: [`docs/evm-rpc-routing.md`](evm-rpc-routing.md)
- Contribution and CI rules: [`AGENTS.md`](../AGENTS.md)
- Root project overview: [`README.md`](../README.md)
- CLI contract: [`bin/cli/README.md`](../bin/cli/README.md)
- REST API contract: [`bin/rest-api/README.md`](../bin/rest-api/README.md)
