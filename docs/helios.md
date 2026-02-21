# Helios in MFM

This document describes the current Helios usage surface in this repository.

## Primary Command Surface

Helios is consumed by the restored Nixfied app:

```bash
export MFM_ENV=dev
export SERVICE_REUSE_POLICY=same-slot
export HELIOS_NETWORK=mainnet
nix run .#mfm::portfolio::snapshot -- <ADDRESS>
```

The wrapper is implemented in `nixfied/project/module.nix` (`task.mfm.portfolio.snapshot`) and enforces:

- exactly one positional address argument
- defaults `HELIOS_NETWORK` to `mainnet` and rejects non-mainnet values
- fallback `HELIOS_EXECUTION_RPC_URL=https://eth.drpc.org` when unset
- Postgres + Helios lifecycle orchestration with reuse policy envs
- fail-fast on mainnet when the project fallback shim binary is configured
- Helios readiness gating that requires:
  - `eth_blockNumber` success
  - `eth_syncing == false` (unless disabled)
  - bounded head lag versus `HELIOS_EXECUTION_RPC_URL`
- raw `mfm_cli` JSON output only on stdout (`--output-format json`)

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

## Runtime Environment

Important Helios env vars:

- `HELIOS_NETWORK`
- `HELIOS_EXECUTION_RPC_URL`
- `HELIOS_CONSENSUS_RPC_URL`
- `HELIOS_CHECKPOINT`
- `HELIOS_READY_TIMEOUT_SECS`
- `HELIOS_READY_INTERVAL_SECS`
- `HELIOS_REQUIRE_SYNC` (`1` default for snapshot task; set `0` to restore basic readiness)
- `HELIOS_SYNC_MAX_LAG_BLOCKS` (`64` default; max tolerated lag vs execution RPC head)

Runtime routing exported by the wrapper:

- `DATABASE_URL=postgresql://postgres:postgres@127.0.0.1:$POSTGRES_PORT/mfm`
- `MFM_EVM_RPC_URL=http://127.0.0.1:$HELIOS_RPC_PORT`
- source-id routing:
  - `MFM_EVM_RPC_SOURCES_JSON`
  - `MFM_EVM_RPC_PREFERRED_ORDER=helios_local`
  - `MFM_EVM_RPC_SOURCE_ID=helios_local`

## Project Wiring

Helios configuration lives in `nixfied/project/conf.nix` under `modules.helios`.
Canonical service metadata now lives in `nixfied/project/conf.nix` under `services.helios`
(`sourceKeys`, `defaultSource`, and port keys).

- `services.helios` in `nixfied/project/module.nix` is sourced from `conf.services.helios` (with module fallbacks).
- If `pkgs.helios` is unavailable, project config falls back to `nixfied/project/helios-package.nix`.

Note for `mfm::portfolio::snapshot`: it intentionally keeps a custom Helios readiness loop because
it defaults to a remote execution RPC (`HELIOS_EXECUTION_RPC_URL=https://eth.drpc.org`) while
framework `ready/health` for `helios` currently assume local execution RPC port checks.

This keeps Helios packaging in project-owned files and avoids project-layer references to framework-private paths.
