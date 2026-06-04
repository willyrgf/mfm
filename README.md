# MFM

Experimental, WIP toolkit for on-chain operations built around an event-sourced state machine runtime.

> WARNING: Not production-ready. Do not use on mainnet.

## Architecture at a glance

```mermaid
flowchart TD
    B["bin/cli<br/>bin/rest-api<br/>(transport only)"] --> A["crates/app<br/>(typed assembly)"]
    A --> O["crates/ops/*-op<br/>(typed program planning)"]
    O --> SH["crates/states/*<br/>(typed executable states)"]
    A --> R["crates/kernel/runtime<br/>(typed scheduler)"]
    R --> K["crates/kernel/*<br/>(ids, values, spec, events, store, replay)"]
    R --> ST["crates/storages/*<br/>(mfm-store implementations)"]
    R --> TP["crates/transports/*<br/>(typed capability backends)"]
    C["crates/core<br/>(primitives + keystore/crypto)"] --> SH

    classDef transport fill:#e8f0ff,stroke:#2f5aa8,color:#0f2d63,stroke-width:1px;
    classDef orchestration fill:#eefbe7,stroke:#3a7a2a,color:#1d4d12,stroke-width:1px;
    classDef statecore fill:#ffe8cf,stroke:#a84b00,color:#5a2b00,stroke-width:3px;
    classDef engine fill:#fff3df,stroke:#a66a00,color:#5a3a00,stroke-width:1px;
    classDef storage fill:#f3ebff,stroke:#6d3da8,color:#39136b,stroke-width:1px;
    classDef adapter fill:#e9f8f7,stroke:#0d7a77,color:#084645,stroke-width:1px;

    class B transport;
    class A,O orchestration;
    class SH statecore;
    class R,K,C engine;
    class ST storage;
    class TP adapter;
```

Typed state programs are the semantic executable surface. Ops plan typed programs, the certified typed execution spec is the runtime contract, states execute through typed effect runners and capability backends, and binaries stay transport-only.

## Core capabilities

- Event-sourced certified typed runs with append-only execution history.
- Crash-resume and replay-aware typed execution semantics.
- Content-addressed manifests, snapshots, facts, and outputs.
- Deterministic typed-state orchestration for ops/pipelines.
- Thin CLI and REST transport layers for stable automation surfaces.
- Security-hardened Ethereum keystore (tamper checks + signing utilities).
- Typed storage backends for run events and artifacts.

## Documentation

Start here:

- Design contract (source of truth): [`docs/design.md`](docs/design.md)
- Architecture taxonomy + placement rules: [`docs/architecture.md`](docs/architecture.md)
- Contribution rules / CI parity: [`AGENTS.md`](AGENTS.md)
- Code quality policy: [`docs/code-quality.md`](docs/code-quality.md)

User-facing docs:

- CLI docs + output contract: [`bin/cli/README.md`](bin/cli/README.md)
- REST API docs: [`bin/rest-api/README.md`](bin/rest-api/README.md)

Crate docs:

- Core primitives (keystore + config models): [`crates/core/README.md`](crates/core/README.md)
- Typed runtime: [`crates/kernel/runtime/README.md`](crates/kernel/runtime/README.md)
- Typed store contract: [`crates/kernel/store/README.md`](crates/kernel/store/README.md)
- Typed replay: [`crates/kernel/replay/README.md`](crates/kernel/replay/README.md)
- Ops (proof op): [`crates/ops/proof-op/README.md`](crates/ops/proof-op/README.md)
- Storage (typed run events, Postgres): [`crates/storages/stream-store-postgres/README.md`](crates/storages/stream-store-postgres/README.md)
- Storage (typed artifacts, fs): [`crates/storages/artifact-store-fs/README.md`](crates/storages/artifact-store-fs/README.md)

Design notes / planning:

- Nixfied vendoring boundaries: [`nixfied/VENDORED.txt`](nixfied/VENDORED.txt)
- Framework upgrade notes: [`docs/UPGRADE.md`](docs/UPGRADE.md)

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

- Framework service hooks and model-level service tasks share the same launcher/runtime policy path, but this repo does not currently expose public `service::*::start` apps.
- Local supervisor wrappers in `nixfied/local/default.nix` (`up`, `down`, `svc-*`) are intentional prod-only overrides.
- `mfm::portfolio::snapshot` is a strict json app that owns process/bootstrap orchestration, stdout envelope
  validation, and readiness mediation via `SVC_*` hooks while forwarding validated `mfm_cli` payloads unchanged.
- `nix run .#help` lists exposed core apps; invoke `mfm::portfolio::snapshot` directly by name.
- CI help/docs metadata comes from task/app metadata in `nixfied/project/module.nix`, and is mirrored into `apps.<system>.ci.meta.nixfied.api`.
- Project scripts should prefer framework policy helpers and `task.ops.ready`/`task.ops.health` surfaces over duplicating service policy or readiness logic.
- `nix run .#dev` intentionally uses `start_service ... --cleanup` for deterministic teardown.

### Reliability improvements:

- Explicit shell app classes are now enforced:
  - `typed` for strict text commands (`check`, `test`, `build`, `mfm_rest_api`, `svc-*`)
  - `json` for machine-output workflows (`mfm::portfolio::snapshot`, and other machine-output apps)
  - `batch-runner` for `ci`
- Unknown args are rejected at the shell-contract boundary for typed/json commands before domain execution.
- `mfm::portfolio::snapshot` preserves `mfm_cli` as the output-contract authority and returns the validated
  `mfm_cli` payload unchanged.
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
- Service model surfaces:
  - `nix run .#services`
  - `nix run .#ready -- --service postgres --source local`
  - `nix run .#ready -- --service reth --source local`
  - `nix run .#health -- --service all`
- Policy controls (optional overrides):
  - `SERVICE_OWNER_SCOPE=ephemeral|persistent`
  - `SERVICE_DISCOVERY_SCOPE=local|global`
- Migration note:
  - `SERVICE_REUSE_POLICY` and `MFM_KEEP_SERVICES` are rejected by `mfm::portfolio::snapshot`.
- Preserve or tear down snapshot-managed services via `SERVICE_*` policy envs on `mfm::portfolio::snapshot`; the direct-task
  launch and lifecycle handling remain internal.

### Run binaries:

```bash
nix build .#mfm-cli
./result/bin/mfm_cli --help
nix run .#mfm_rest_api
nix run .#mfm::portfolio::snapshot -- <ADDRESS>
```

## License
MIT (see [`LICENSE`](LICENSE)).
