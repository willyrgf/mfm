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
- `HELIOS_NETWORK=mainnet` (hard precondition)
- fallback `HELIOS_EXECUTION_RPC_URL=https://eth.drpc.org` when unset
- Postgres + Helios lifecycle orchestration with reuse policy envs
- Helios readiness gating via RPC `eth_blockNumber` probe loop
- raw `mfm_cli` JSON output only on stdout (`--output-format json`)

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

Runtime routing exported by the wrapper:

- `DATABASE_URL=postgresql://postgres:postgres@127.0.0.1:$POSTGRES_PORT/mfm`
- `MFM_EVM_RPC_URL=http://127.0.0.1:$HELIOS_RPC_PORT`
- source-id routing:
  - `MFM_EVM_RPC_SOURCES_JSON`
  - `MFM_EVM_RPC_PREFERRED_ORDER=helios_local`
  - `MFM_EVM_RPC_SOURCE_ID=helios_local`

## Project Wiring

Helios configuration lives in `nixfied/project/conf.nix` under `modules.helios`.

- `services.helios.enable` in `nixfied/project/module.nix` follows `conf.modules.helios.enable`.
- If `pkgs.helios` is unavailable, project config falls back to `nixfied/project/helios-package.nix`.

This keeps Helios packaging in project-owned files and avoids project-layer references to framework-private paths.
