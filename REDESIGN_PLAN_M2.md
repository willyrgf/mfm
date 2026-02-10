# MFM — `REDESIGN.md` Implementation Plan (Milestone 2)

> Source of truth: `REDESIGN.md` (v4, last updated 2026-02-09).
> Generated: 2026-02-10
>
> Goal: add real IO namespaces + a first real vertical op (EVM read-only), without changing the Milestone 1
> engine/public API contracts.

---

## Summary

Milestone 2 delivers:

1. A new `crates/collectors/evm/` crate that defines typed adapters on top of `IoCall` for JSON-RPC EVM reads.
2. A `LiveIoTransport` router so multiple namespaces can coexist (e.g., `proof.*`, `evm`, `exec`).
3. A first non-test vertical op: `evm_read` (read-only, deterministic via recorded facts).
4. An `exec` namespace foundation (for pinned `/nix/store/...` program execution) plus a `nix_app` op
   (per `NIX_STATE_PROPOSAL.md`).

Milestone 2 MUST keep Milestone 1 invariants:
- append-only kernel envelopes + resume correctness
- canonical JSON hashing rules (no floats)
- no secrets persisted in manifests/events/artifacts/snapshots/outputs/errors
- no ambient IO in state handlers (IO only via `IoProvider`)

---

## Locked Decisions

- IO namespaces:
  - EVM JSON-RPC: `namespace = "evm"`
  - External exec: `namespace = "exec"`
- Fact keys for collector IO MUST be explicit and stable (not attempt/ordinal-derived).
- The EVM op MUST NOT require secret-bearing config in `op_config_json`.
  - The CLI may accept secret-bearing live configuration (e.g. RPC URL) but MUST NOT persist it into the manifest.
- Store large raw payloads as fact payload artifacts; context stores only small derived values and artifact IDs.

---

## Deliverables (Step-by-Step)

### M2-01 — Scaffold `crates/collectors/evm`

Tasks:
- Add `crates/collectors/evm/Cargo.toml` + `src/lib.rs`.
- Provide typed helpers:
  - `JsonRpcCall { method, params }` (serde)
  - `evm_io_call(call: JsonRpcCall, fact_key: FactKey) -> IoCall`
  - `fact_key_for_jsonrpc(op_path, method, params) -> FactKey` derived from canonical-json hash
  - Response parsing helpers:
    - `parse_u64_hex("0x...") -> Result<u64, Error>`
    - `extract_jsonrpc_u64(result_value) -> Result<u64, Error>`

Acceptance:
- Unit tests for:
  - fact key stability for same `(method, params)`
  - `parse_u64_hex` correctness and error cases
- `cargo test -p mfm-collectors-evm`

### M2-02 — Add a `LiveIoTransport` router (multi-namespace)

Tasks:
- Implement `mfm_machine::live_io_router` (NOT stable API):
  - `RouterTransportFactory { routes: HashMap<String, Arc<dyn LiveIoTransportFactory>> }`
  - `RouterTransport { routes: HashMap<String, Box<dyn LiveIoTransport>> }`
  - Unknown namespace returns a stable `IoError` without echoing request payloads (avoid accidental secrets).

Acceptance:
- Unit tests:
  - dispatches to the correct route
  - missing route returns error code `io_unknown_namespace`
- `cargo test -p mfm-machine`

### M2-03 — Implement `crates/ops/evm-read-op` (first vertical op)

Operation contract:
- `op_id = "evm_read"`
- `op_version = "v1"`
- `op_config` (canonical JSON, no secrets):
  - `{ "include_chain_id": true, "include_block_number": true }` (both default true if omitted)

Expansion:
- Expand to a small `StateGraph` with two sequential states:
  - `<op_path>.chain_id` calls `eth_chainId`, writes `<op_path>.chain_id = u64`
  - `<op_path>.block_number` calls `eth_blockNumber`, writes `<op_path>.block_number = u64`
- Both states:
  - `SideEffectKind::ReadOnlyIo`
  - stable explicit `FactKey` derived from `(op_path, method, params)`

Acceptance:
- Add op-level acceptance tests (like `mfm-op-proof`):
  - live→replay determinism: replay reproduces final snapshot hash
  - crash/resume determinism: inject orphan attempt after first handler and resume; transport is not called twice
- `cargo test -p mfm-op-evm-read`

### M2-04 — Implement `exec` transport foundation + `nix_app` op

Tasks:
- Add `crates/ops/nix-app-op`:
  - `op_id = "nix_app"`, `op_version = "v1"`
  - expands to a single state `<op_path>.run` that calls `namespace="exec"` with a stable fact key
  - request/response contract per `NIX_STATE_PROPOSAL.md`
- Add `exec` live transport (implementation location is flexible):
  - Takes `program_path`, `argv`, `stdin_json`, `timeout_ms`, `env_allowlist`
  - Executes via `tokio::process::Command`
  - Parses stdout as JSON result (stderr is not persisted)
  - Error messages MUST NOT include stdout/stderr (avoid secret leakage)

Acceptance:
- Fast lane tests: fake exec transport records facts and replays deterministically.
- Parity lane tests (opt-in): run a pinned `/nix/store/...` program if available.

### M2-05 — Wire CLI to registry + transport router

Tasks:
- Register `evm_read` and `nix_app` in `bin/cli` operation registry.
- Replace single transport with router:
  - keep `proof.*` namespaces working
  - keep unknown namespace errors stable

Acceptance:
- `cargo test -p mfm` (CLI crate) and existing e2e tests remain green.

---

## CI / Done Definition

Milestone 2 is “done” when:
- all new crates compile and tests pass
- proof acceptance tests still pass
- EVM read op tests demonstrate:
  - live→replay determinism
  - crash/resume determinism with no double IO execution
- no new persisted-surface secret leaks are introduced

