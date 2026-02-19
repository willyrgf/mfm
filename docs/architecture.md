# MFM Architecture Guide

> Contributor-facing architecture overview.
> Normative design contract: [`docs/redesign.md`](redesign.md).

## 1. How To Use This Document

Use this guide to decide where code belongs and what invariants must hold.

Read order for contributors:
1. This document (`docs/architecture.md`) for boundaries and implementation placement.
2. [`docs/redesign.md`](redesign.md) for normative execution and storage semantics.
3. [`AGENTS.md`](../AGENTS.md) for workflow, CI parity, and contribution rules.

If code conflicts with `docs/redesign.md`, treat that as a contract violation until the contract is intentionally updated.

## 2. MFM In 5 Minutes

MFM is an event-sourced execution engine for reproducible workflows.

A run is defined by three immutable surfaces:
- A content-addressed manifest.
- An append-only event stream.
- Content-addressed artifacts (snapshots, facts, outputs).

Execution unit model:
- Ops expand into state graphs.
- The runtime executes states.
- CLI/REST start or resume runs and render outputs.

## 3. Non-Negotiable Invariants

1. Append-only event streams.
- Historical events are never mutated.

2. Per-append atomicity.
- Each `EventStore::append([...])` is all-or-nothing.

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
- Secrets must never appear in manifests, events, artifacts, snapshots, CLI/API outputs, or error details.

9. Thin binary boundary.
- Domain execution logic belongs in ops/state-machine layers, not in `bin/cli` or `bin/rest-api`.

10. EVM network transport boundary.
- Runtime EVM network calls (`namespace: "evm"`) must route through
  `mfm-collectors-evm-jsonrpc-http` wiring in `crates/app`.
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
The canonical architecture follows the three-tier model from `docs/three-tier-audit.md`:

```text
bin/{cli,rest-api}                 (THIN) transport parsing/routing/output only
          |
          v
     crates/ops/*                  (THIN) config validation + state graph wiring
          |
          v
shared state crates                (THICK) reusable single-responsibility states
  crates/ops/common/src/states/*
  crates/ops/keystore-common/src/states/*
  crates/ops/aave-v3-common/src/*
  crates/evm-runtime/src/states/*
```

Core rule:
- executable logic lives in reusable states
- ops assemble state graphs
- binaries stay transport-only

Allowed exception:
- op-local state implementations are acceptable only for domain-specific output/aggregation states
  that are not reusable shared primitives.

### Ops are planning abstractions
- An op resolves to a concrete `StateGraph` given `OpConfig` and `RunConfig`.
- `expand()` must be deterministic and must not perform IO.

### Flattened pipelines
- Multiple ops are flattened into one execution plan and one run.
- Shared context is namespaced; cross-op imports/exports are explicit.

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
- launch/resume glue over machine + stores
- machine-friendly pipeline interfaces

Must stay thin and avoid domain-specific execution behavior.

### `crates/ops/common/`
Owns cross-domain reusable state primitives and shared op-level helpers.
Domain-specific reusable states may live in dedicated shared-state crates
(`crates/ops/keystore-common`, `crates/ops/aave-v3-common`, `crates/evm-runtime`).

### `crates/ops/*`
Owns domain workflows:
- operation config validation
- graph composition using reusable states
- op-level tests and domain contracts
Prefer thin ops: avoid embedding thick executable logic when a reusable state belongs in a shared-state crate.

### Breaking Internal Refactors
Internal crate/module path migrations may be performed as hard cutovers:
- no compatibility re-exports
- no deprecated aliases
- all in-repo consumers updated in the same change

### `crates/collectors/*`
Owns external data collection and normalization adapters.

### `crates/storages/*`
Owns event/artifact persistence implementations only.

### `crates/core/`
Owns primitives and security-sensitive keystore/crypto code.

### `bin/cli/` and `bin/rest-api/`
Own transport adaptation only:
- parse requests
- invoke run start/resume/query APIs
- render stable output contracts

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
- Is it business workflow execution? Put it in `crates/ops/*`.
- Is it reusable workflow state logic? Put it in `crates/ops/common/`.
- Is it runtime semantics? Put it in `crates/machine/`.
- Is it transport parsing/rendering only? Put it in `bin/cli` or `bin/rest-api`.

If the answer spans multiple layers, split responsibilities explicitly rather than letting binaries absorb domain logic.

## 12. Related Documents

- Normative contract: [`docs/redesign.md`](redesign.md)
- EVM routing runbook: [`docs/evm-rpc-routing.md`](evm-rpc-routing.md)
- Contribution and CI rules: [`AGENTS.md`](../AGENTS.md)
- Root project overview: [`README.md`](../README.md)
- CLI contract: [`bin/cli/README.md`](../bin/cli/README.md)
- REST API contract: [`bin/rest-api/README.md`](../bin/rest-api/README.md)
