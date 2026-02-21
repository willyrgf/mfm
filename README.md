# MFM

Experimental, WIP toolkit for on-chain operations built around an event-sourced state machine runtime.

> WARNING: Not production-ready. Do not use on mainnet.

## Architecture at a glance

```mermaid
flowchart TD
    B["bin/cli<br/>bin/rest-api<br/>(transport only)"] --> S["crates/sdk<br/>(run start/resume glue)"]
    S --> O["crates/ops/*-op<br/>(op config + graph composition)"]
    O --> SH["shared state layer<br/>crates/states/common<br/>crates/states/keystore<br/>crates/states/aave-v3<br/>crates/evm-runtime/src/states/*"]
    SH --> M["crates/machine<br/>(runtime/replay/resume)"]
    M --> ST["crates/storages/*<br/>(event/artifact persistence)"]
    K["crates/core<br/>(primitives + keystore/crypto)"] --> SH
    AD["crates/collectors/*<br/>(typed domain adapters over IoProvider)"] --> SH
    LT["live transport factories<br/>crates/collectors/* + crates/transports/*"] --> M

    classDef transport fill:#e8f0ff,stroke:#2f5aa8,color:#0f2d63,stroke-width:1px;
    classDef orchestration fill:#eefbe7,stroke:#3a7a2a,color:#1d4d12,stroke-width:1px;
    classDef statecore fill:#ffe8cf,stroke:#a84b00,color:#5a2b00,stroke-width:3px;
    classDef engine fill:#fff3df,stroke:#a66a00,color:#5a3a00,stroke-width:1px;
    classDef storage fill:#f3ebff,stroke:#6d3da8,color:#39136b,stroke-width:1px;
    classDef adapter fill:#e9f8f7,stroke:#0d7a77,color:#084645,stroke-width:1px;

    class B transport;
    class S,O orchestration;
    class SH statecore;
    class M,K engine;
    class ST storage;
    class AD,LT adapter;
```

States are the core execution unit. Ops are planners (`expand` builds deterministic graphs); shared states are executors (`handle` performs one step via `IoProvider`), and binaries stay transport-only.

## Core capabilities

- Event-sourced runs with append-only execution history.
- Crash-resume and replay-aware execution semantics.
- Content-addressed manifests, snapshots, facts, and outputs.
- Deterministic state-machine orchestration for ops/pipelines.
- Thin CLI and REST transport layers for stable automation surfaces.
- Security-hardened Ethereum keystore (tamper checks + signing utilities).
- Swappable storage backends (in-memory, Postgres, fs, S3/MinIO).

## Documentation

Start here:

- Design contract (source of truth): [`docs/redesign.md`](docs/redesign.md)
- One-page overview + invariants: [`docs/architecture.md`](docs/architecture.md)
- Contribution rules / CI parity: [`AGENTS.md`](AGENTS.md)

User-facing docs:

- CLI docs + output contract: [`bin/cli/README.md`](bin/cli/README.md)
- REST API docs: [`bin/rest-api/README.md`](bin/rest-api/README.md)

Crate docs:

- Core primitives (keystore + config models): [`crates/core/README.md`](crates/core/README.md)
- Runtime: [`crates/machine/README.md`](crates/machine/README.md)
- Proc macros: [`crates/machine-derive/README.md`](crates/machine-derive/README.md)
- SDK (orchestration helpers): [`crates/sdk/README.md`](crates/sdk/README.md)
- Ops (proof op): [`crates/ops/proof-op/README.md`](crates/ops/proof-op/README.md)
- Ops (keystore op): [`crates/ops/keystore-op/README.md`](crates/ops/keystore-op/README.md)
- Storage (EventStore, mem): [`crates/storages/event-store-mem/README.md`](crates/storages/event-store-mem/README.md)
- Storage (EventStore, Postgres): [`crates/storages/event-store-postgres/README.md`](crates/storages/event-store-postgres/README.md)
- Storage (ArtifactStore, fs): [`crates/storages/artifact-store-fs/README.md`](crates/storages/artifact-store-fs/README.md)
- Storage (ArtifactStore, S3/MinIO): [`crates/storages/artifact-store-s3/README.md`](crates/storages/artifact-store-s3/README.md)

Design notes / planning:

- Nixfied vendoring boundaries: [`nixfied/VENDORED.txt`](nixfied/VENDORED.txt)

## Development

Canonical entrypoints:

```bash
nix run .#help
nix run .#dev
nix run .#check
nix run .#test
nix run .#ci -- --mode basic --summary
nix run .#ci -- --mode audit --summary
nix run .#ci -- --mode parity --summary
nix run .#ci -- --mode full --summary
nix run .#ci -- --mode <mode> --summary
```

### Behavioral changes (workflow modes)

- `--mode <value>` is the canonical interface for workflow mode selection.
- Shorthand `--<mode>` is only accepted for simple mode names matching `[a-z0-9-]+`.
- Dotted/complex modes must use `--mode` (for example `--mode parallel.smoke`).
- Unknown mode errors now print expected modes derived from the compiled model.

### Contract notes:

- Service hooks (for example `run_hook MINIO_START`) and service apps (for example `nix run .#service::minio::start`) share the same launcher path and argument/slot-env enforcement.
- Local supervisor wrappers in `nixfied/local/default.nix` (`up`, `down`, `svc-*`) are intentional prod-only overrides.
- `mfm_cli` is the compatibility passthrough wrapper; `mfm_rest_api`/`svc-*` now use strict typed contracts.
- `mfm::portfolio::snapshot` is restored as a strict json app that orchestrates Postgres + Helios and emits raw `mfm_cli` JSON payloads.
- `nix run .#help` lists exposed core apps; invoke `mfm::portfolio::snapshot` directly by name.
- CI help/docs metadata comes from `nixfied/project/ci.nix` at `commands.ci.api`, and is mirrored into `apps.<system>.ci.meta.nixfied.api`.
- Project scripts should prefer framework policy helpers (for example `start_service_should_register_cleanup`) over duplicating `SERVICE_*` policy matrix logic.
- `nix run .#dev` intentionally uses `start_service ... --cleanup` for deterministic teardown.

### Reliability improvements:

- Explicit shell app classes are now enforced:
  - `typed` for strict text commands (`check`, `test`, `build`, `mfm_rest_api`, `svc-*`)
  - `json` for machine-output workflows (`mfm::portfolio::snapshot`, and other machine-output apps)
  - `batch-runner` for `ci`
  - `passthrough` retained for compatibility on `mfm_cli`
- Unknown args are rejected at the shell-contract boundary for typed/json commands before domain execution.
- `mfm::portfolio::snapshot` preserves `mfm_cli` as the output-contract authority and returns the raw JSON payload unchanged.
- CI now validates command class policies and runtime behavior (including unknown-arg probes and JSON-shape checks) in `shell-app-contracts`.

### Process-first ops:

- Runtime visibility:
  - `nix run .#process::status`
  - `nix run .#process::status -- --all`
  - `nix run .#process::runs -- --all`
  - `nix run .#process::inspect -- <id>`
  - `nix run .#process::stop -- --run-id <id>`
  - `nix run .#process::stop -- --run-id <id> --scope slot-env`
  - `nix run .#process::stop -- --run-id <id> --dry-run`
- Service diagnostics:
  - `nix run .#service::postgres::events -- --limit 100`
  - `nix run .#service::postgres::log -- --lines 200`
  - `nix run .#service::helios::events -- --limit 100`
- Policy controls (optional overrides):
  - `SERVICE_REUSE_POLICY=never|same-root|same-slot|cross-run`
  - `SERVICE_OWNER_SCOPE=ephemeral|persistent`
  - `SERVICE_DISCOVERY_SCOPE=local|global`
- Migration note:
  - `MFM_KEEP_SERVICES` has been removed from `mfm::portfolio::snapshot`.
  - Start reusable services explicitly via `service::*::start`, then inspect ownership with `process::status`.

### Run binaries:

```bash
nix run .#mfm_cli -- --help
nix run .#mfm_rest_api
nix run .#mfm::portfolio::snapshot -- <ADDRESS>
```

## License
MIT (see [`LICENSE`](LICENSE)).
