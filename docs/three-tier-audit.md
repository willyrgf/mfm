# Three-Tier Thin-Layer Alignment Audit

Date: 2026-02-14
Status: Historical checkpoint after the thin-layer refactor. State extraction and the state-layer path migration are complete; the current approved shared-state roots span `crates/states/*` and `crates/evm-runtime/src/states/*`, and any remaining follow-up items should be read in that context.

For the current catalog of registered ops and production states, see
[`docs/ops-and-states.md`](ops-and-states.md). This file is a 2026-02-14 checkpoint.

## 0. Architectural Model

```
bin/{cli,rest-api}   (THIN) parse input, select op/pipeline, render output
       |
       v
  crates/ops/*-op    (THIN) config validation + state graph wiring only
       |
       v
shared-state crates  (THICK) reusable single-responsibility states
  crates/states/common/src/states/*
  crates/states/keystore/src/states/*
  crates/states/aave-v3/src/states/*
  crates/evm-runtime/src/states/*
  (+ shared pure utility modules)
       |
       v
adapters/IO drivers  collectors, local_io, storages
```

Design rule: executable logic lives in reusable states; ops assemble state graphs; binaries stay transport-only.

## 1. Checkpoint Snapshot (Historical)

The metrics below are frozen as of 2026-02-14. They are checkpoint evidence, not the current
inventory.

| Metric | Current |
|---|---:|
| CLI command surfaces audited | 13 |
| REST routes audited | 9 |
| Registered runtime ops | 14 |
| Shared production `State` impls (`states/common/src/states`) | 18 |
| Op-local production `State` impls (`ops/*/src/lib.rs`) | 2 |
| Total production `State` impls | 20 |
| Shared state ratio (production) | 90% |
| Target shared ratio | 80%+ |

Notes:
- Shared ratio now exceeds target.
- The two intentional op-local states are domain-specific output aggregators.

## 2. Tier Conformance

### 2.1 Tier 1: Binary Layer (Thin)

- REST API: aligned (wrapper-only handlers).
- CLI: aligned for thin-layer boundary.
  - `portfolio snapshot` token JSON parsing moved into app-layer helper.
  - `run pipeline deploy-configure-validate` spec input parsing moved into app-layer helper.
  - Remaining CLI JSON parsing for generic pipeline payloads is transport-level request decoding.

### 2.2 Tier 2: Ops Layer (Thin)

Historical summary at this checkpoint:

- `12/14` ops were thin
- `2/14` ops were mixed
- `0/14` ops were thick

For the current built-in op catalog and owning crates, see [`docs/ops-and-states.md`](ops-and-states.md).

### 2.3 Tier 3: State Layer (Thick + Reusable)

Historical summary at this checkpoint:

- reusable state extraction had completed for keystore, EVM read/write, nix exec, and proof patterns
- a small number of op-local output aggregation states were still retained by design

For the current production state inventory and current exceptions, see
[`docs/ops-and-states.md`](ops-and-states.md).

## 3. Utility Consolidation (`B8`)

Canonical shared utility modules are in place:
- `crates/states/common/src/hex.rs`
- `crates/states/common/src/rlp.rs`
- `crates/states/common/src/abi.rs`
- `crates/states/common/src/evm_encoding.rs`
- `crates/states/common/src/util_error.rs`

Interpretation:
- RLP duplication is now single-source.
- Remaining duplication is mostly wrapper-layer compatibility around shared helpers.
- Final strict-mode target remains single-source for hex/ABI paths as well.

## 4. Guardrails and CI

`mfm-architecture-verify` was an interim guardrail during the thin-layer and compile-time boundary refactors. It was later retired once its remaining checks were enforced more directly by compile-time boundaries and crate structure.

Current guardrails come from:
1. The `Operation::expand()` contract in `crates/sdk/src/lib.rs` remains synchronous and deterministic, so `.await` is already a type error there.
2. `clippy.toml` now bans raw `IoProvider::call` and other ambient APIs from planner/state-facing crates, leaving direct EVM bridge construction confined to the dedicated collector path.
3. Keystore signing and remote submission now live in separate crates, so the old mixed local-signing plus remote-submit path is enforced structurally instead of by a text scan.

## 5. Findings (Current)

### High

1. `B8` strict single-source utility goal is not fully complete.
- Impact: wrapper duplications remain for select hex/ABI helpers.
- Risk: minor maintenance drift risk compared to fully canonical call sites.

### Medium

2. Shared test harness duplication remains.
- `NoopRecorder` and `CountingTransport` are still duplicated in multiple test modules.
- `MapContext` still has duplicate implementations outside shared test support.

3. Full parity signoff remains a release gate item.
- `#check` and `#test` pass.
- `#ci -- --parity --summary` should be run for architecture checkpoint closure.

### Low

4. Two ops remain mixed by design (`portfolio_tracker`, `proof`) due domain-specific output assembly states.

## 6. Remaining Plan

1. Tighten utility duplication policy from ceiling mode to strict single-source mode (hex/ABI wrappers).
2. Consolidate duplicated test doubles into shared test support.
3. Run and archive parity CI summary (`nix run .#ci -- --parity --summary`).
4. Update architecture-facing docs (`docs/architecture.md`, `docs/redesign.md`, `AGENTS.md`) if needed to reflect new shared state modules.

## 7. Acceptance Criteria Status

1. No business workflow logic in binaries: **Met**.
2. Op `expand()` methods are config+graph only: **Met**.
3. Shared-state extraction complete: **Met**. The old ratio metric remains historical context only and is no longer a hard architecture gate.
4. Op-local states only for domain-specific aggregation/output: **Met**.
5. Reusable patterns extracted as shared states: **Met** for keystore-tx, evm-write, proof patterns, nix exec.
6. Side effects routed through IO abstraction: **Met**.
7. Architecture guard in CI: **Met**.
8. `check`/`test` parity gates passing: **Met** for `#check` and `#test`; full parity mode still pending in this checkpoint.
9. Utility single-source (strict): **Partially met** (RLP complete, hex/ABI wrappers remain).
10. Shared test harness for state tests: **Partially met**.
11. Utility test coverage for consolidated modules: **Partially met**.
12. Execute tests for every shared state: **Partially met**.
