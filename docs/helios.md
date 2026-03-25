# Helios in MFM

This document describes the current Helios usage surface in this repository.

## Primary Command Surface

Helios is consumed by the restored Nixfied app:

```bash
export MFM_ENV=dev
export SERVICE_OWNER_SCOPE=persistent
export SERVICE_DISCOVERY_SCOPE=global
export HELIOS_NETWORK=mainnet
nix run .#mfm::portfolio::snapshot -- ./portfolio-request.json
```

The public wrapper lives in `nixfied/project/module.nix` as `task.mfm.portfolio.snapshot`.
It owns stdout and directly:

- validates the request-file contract
- manages Postgres and Helios lifecycle through `SVC_POSTGRES_*` / `SVC_HELIOS_*` hooks
- runs the packaged `mfm_cli --output-format json portfolio snapshot --request-file ...`
- validates the returned JSON envelope before replaying it to stdout

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

- `SERVICE_OWNER_SCOPE=ephemeral|persistent`
- `SERVICE_DISCOVERY_SCOPE=local|global`

`SERVICE_REUSE_POLICY` and legacy `MFM_KEEP_SERVICES` are rejected by this command.

The helper tasks are internal implementation details. This repo does not expose
custom project-owned `service::*::start` wrappers for Postgres or Helios.
Lifecycle now flows through the vendored Nixfied service primitives:

- `svc::postgres::*`
- `svc::helios::*`

## Runtime Environment

Important Helios env vars:

- `HELIOS_NETWORK`
- `HELIOS_EXECUTION_RPC_URL`
- `HELIOS_CONSENSUS_RPC_URL`
- `HELIOS_DEFAULT_CONSENSUS_RPC_URL`
- `HELIOS_CHECKPOINT`
- `HELIOS_READY_TIMEOUT_SECS`
- `HELIOS_READY_INTERVAL_SECS`

Model-derived executor env uses `HELIOSRPC_PORT`. Internal snapshot tasks still
resolve the canonical model port keys directly; the public snapshot task uses
`HELIOSRPC_PORT` for its managed local Helios wiring.

Runtime bootstrap inside `task.mfm.portfolio.snapshot`:

- `DATABASE_URL=postgresql://postgres:postgres@127.0.0.1:$POSTGRES_PORT/mfm`
- request file passed as `--request-file /absolute/path/to/portfolio-request.json`
- managed RPC bootstrap:
  - `MFM_EVM_RPC_SOURCES_JSON=[{"id":"helios_local","network_id":"ethereum-mainnet","rpc_url":"http://127.0.0.1:$HELIOSRPC_PORT","kind":"local"}]`
  - `MFM_EVM_RPC_PREFERRED_ORDER=helios_local`

`mfm_cli` is no longer launched via `cargo run` from the snapshot path. It is
packaged once through `conf.packages."mfm-cli"` and reused by both
`nix run .#mfm_cli` and `task.mfm.portfolio.snapshot`.

## Project Wiring

Helios configuration lives in `nixfied/project/conf.nix` under `services.helios`
(`sources`, `defaultSource`, `sourceKinds`, readiness policy, port keys, and
network defaults).

- `services.helios` in `nixfied/project/module.nix` is sourced directly from `conf.services.helios`.
- `services.helios.sources.local.package` is the authoritative local Helios package setting.
- The local source is explicitly marked `real`, and framework readiness uses the `strict`
  profile so `ready -- --service helios --source local` rejects shim or unknown source kinds.

`task.mfm.portfolio.snapshot` performs lifecycle orchestration directly through
`SVC_POSTGRES_*` / `SVC_HELIOS_*` hooks exported by Nixfied instead of routing
through a project-local workflow split or importing service scripts directly
into `nixfied/project/module.nix`.

The parity CI path also uses `SVC_HELIOS_FULL_START_TEST`, `SVC_HELIOS_READY`,
and `SVC_HELIOS_STOP`; the repo no longer carries the previous Python Helios
shim for parity startup. Parity clears explicit `HELIOS_CONSENSUS_RPC_URL` /
`HELIOS_CHECKPOINT` overrides and relies on the service-level
`HELIOS_DEFAULT_CONSENSUS_RPC_URL` fallback instead.

Execution endpoint inputs resolve in this order:

1. `HELIOS_EXECUTION_RPC_URL` (environment override)
2. `services.helios.executionRpcUrl` (project config)
3. local fallback from `services.helios.executionRpcPortKey`
