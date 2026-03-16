# Helios in MFM

This document describes the current Helios usage surface in this repository.

## Primary Command Surface

Helios is consumed by the restored Nixfied app:

```bash
export MFM_ENV=dev
export SERVICE_REUSE_POLICY=same-slot
export HELIOS_NETWORK=mainnet
nix run .#mfm::portfolio::snapshot -- ./portfolio-request.json
```

The public wrapper lives in `nixfied/project/module.nix` as `task.mfm.portfolio.snapshot`.
It owns stdout and delegates sequencing to the internal workflow:

- `workflow.mfm.portfolio.snapshot`

That workflow currently composes hidden implementation tasks:

- `task.mfm.portfolio.snapshot.services-start`
- `task.mfm.portfolio.snapshot.exec`
- `task.mfm.portfolio.snapshot.services-stop`

The public task remains the stdout authority. It enforces:

- exactly one positional request-file argument
- a readable canonical portfolio snapshot request JSON file
- defaults `HELIOS_NETWORK` to `mainnet` and rejects non-mainnet values
- Postgres + Helios lifecycle orchestration with `SERVICE_*` policy envs
- workflow sequencing with preflight, service start, packaged CLI execution, and always-run teardown
- validated `mfm_cli --output-format json` output only on stdout

The wrapper validates the JSON envelope before replaying it to the original stdout:

- `.status == "success"`
- `.data.feature_id == "portfolio.snapshot"`
- `.data.result` exists

Framework-level service checks are also available:

```bash
nix run .#services
nix run .#ready -- --service helios --source local
nix run .#health -- --service helios --source local
```

## Lifecycle Controls

`mfm::portfolio::snapshot` respects process policy envs:

- `SERVICE_REUSE_POLICY=never|same-root|same-slot|cross-run`
- `SERVICE_OWNER_SCOPE=ephemeral|persistent`
- `SERVICE_DISCOVERY_SCOPE=local|global`

Legacy `MFM_KEEP_SERVICES` is rejected.

The helper tasks are internal implementation details. This repo does not expose
public `service::*::start` apps for Postgres or Helios.

## Runtime Environment

Important Helios env vars:

- `HELIOS_NETWORK`
- `HELIOS_EXECUTION_RPC_URL`
- `HELIOS_CONSENSUS_RPC_URL`
- `HELIOS_CHECKPOINT`
- `HELIOS_READY_TIMEOUT_SECS`
- `HELIOS_READY_INTERVAL_SECS`

Model-derived executor env uses `HELIOSRPC_PORT`. Internal snapshot tasks export
`HELIOS_RPC_PORT` locally only where Helios process wiring or CLI routing still
expects that alias.

Runtime bootstrap reconstructed inside `task.mfm.portfolio.snapshot.exec`:

- `DATABASE_URL=postgresql://postgres:postgres@127.0.0.1:$POSTGRES_PORT/mfm`
- `MFM_SNAPSHOT_REQUEST_FILE=/absolute/path/to/portfolio-request.json`
- managed RPC bootstrap:
  - `MFM_EVM_RPC_SOURCES_JSON=[{"id":"helios_local","network_id":"ethereum-mainnet","rpc_url":"http://127.0.0.1:$HELIOS_RPC_PORT","kind":"local"}]`
  - `MFM_EVM_RPC_PREFERRED_ORDER=helios_local`

Secret-bearing env is reconstructed per task and is not persisted in the
services handoff file. The handoff JSON stores only non-secret ownership and
path metadata needed for teardown.

`mfm_cli` is no longer launched via `cargo run` from the snapshot path. It is
packaged once through `conf.packages."mfm-cli"` and reused by both
`nix run .#mfm_cli` and `task.mfm.portfolio.snapshot.exec`.

## Project Wiring

Helios configuration lives in `nixfied/project/conf.nix` under `modules.helios`.
Canonical service metadata now lives in `nixfied/project/conf.nix` under `services.helios`
(`sources`, `defaultSource`, `sourceKinds`, readiness policy, and port keys).

- `services.helios` in `nixfied/project/module.nix` is sourced from `conf.services.helios` (with module fallbacks).
- `modules.helios.package` defaults to `pkgs.helios` when available.
- The local source is explicitly marked `real`, and framework readiness uses the `strict`
  profile so `ready -- --service helios --source local` rejects shim or unknown source kinds.

`task.mfm.portfolio.snapshot.services-start` currently remains the coarse lifecycle task.
It reuses framework `task.ops.ready` for both Postgres and Helios, while the
public wrapper keeps stdout clean and the internal workflow manages sequencing.

Execution endpoint inputs resolve in this order:

1. `HELIOS_EXECUTION_RPC_URL` (environment override)
2. `services.helios.executionRpcUrl` (project config)
3. local fallback from `services.helios.executionRpcPortKey`
