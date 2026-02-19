# Helios in MFM (Current Usage + Config)

This document describes how this repository currently uses Helios and which configuration surfaces control it.

## How Helios Is Used Right Now

Helios is integrated as a Nixfied-managed service module, then consumed as a local JSON-RPC endpoint by dev/CI workflows.

- Module wiring:
  - `flake.nix` conditionally imports the Helios framework module when `project.modules.helios.enable` is true (`flake.nix:60`).
  - The service API is exported into framework hooks/apps (`flake.nix:72`).
- Project defaults enable Helios:
  - `project.modules.helios.enable = true` with local network defaults (`nixfied/project/conf.nix:819`).
- Dev workflow consumption:
  - `mfm::portfolio::snapshot` starts/reuses Helios and waits for readiness (`nixfied/project/dev.nix:435`).
  - It configures source-id routing for Helios local RPC via:
    - `MFM_EVM_RPC_SOURCES_JSON`
    - `MFM_EVM_RPC_PREFERRED_ORDER=helios_local`
    - `MFM_EVM_RPC_SOURCE_ID=helios_local`
    - and also sets legacy `MFM_EVM_RPC_URL` compatibility env.
  - This flow is mainnet-only (`HELIOS_NETWORK` must be `mainnet`) (`nixfied/project/dev.nix:331`).
- CI usage:
  - `parity-evm-helios-smoke` starts local Helios over reth execution RPC and performs an `eth_chainId` smoke call (`nixfied/project/ci/scripts/steps/parity-evm-helios-smoke.nix:6`).
  - `mainnet-portfolio-snapshot-helios` runs `mfm::portfolio::snapshot` with mainnet Helios settings (`nixfied/project/ci/scripts/steps/mainnet-portfolio-snapshot-helios.nix:5`).
- Operator diagnostics:
  - Helios events/log surfaces are exposed via service commands (for example in README: `nix run .#service::helios::events -- --limit 100`) (`README.md:127`).

## Configuration Sources and Precedence

Effective runtime values are resolved in this order:

1. Environment variables (`HELIOS_*`) at process runtime.
2. Project module config in `project.modules.helios` (`nixfied/project/conf.nix:819`).
3. Framework module defaults (`nixfied/.framework/helios/config.nix:1`).
4. Runtime derivations/fallbacks in lifecycle scripts (`nixfied/.framework/helios/lifecycle.nix:34`).

## Project Module Configuration (`project.modules.helios`)

Defined in `nixfied/project/conf.nix:819`:

- `enable = true`
- `portKeyRpc = "heliosRpc"`
- `dataDirName = "helios"`
- `network = "local"`
- `executionRpcPortKey = "rethHttp"`
- `executionRpcUrl = "https://eth.drpc.org"`
- `consensusRpcUrl = ""`
- `checkpoint = ""`
- `extraArgs = []`

Related port defaults in the same file:

- `ports.heliosRpc = 8547` (`nixfied/project/conf.nix:611`)
- `ports.rethHttp = 8545` (`nixfied/project/conf.nix:607`)

## Framework Defaults and Runtime Behavior

From `nixfied/.framework/helios/config.nix:1`:

- `defaultConsensusRpcUrl = "https://www.lightclientdata.org"` (used as the mainnet default consensus endpoint).

From `nixfied/.framework/helios/lifecycle.nix:34`:

- Runtime env hydration:
  - `HELIOS_NETWORK` from env or config default.
  - `HELIOS_EXECUTION_RPC_URL` from env/config; if empty and mapped execution port exists, fallback to `http://127.0.0.1:$HELIOS_EXECUTION_PORT`.
  - `HELIOS_CONSENSUS_RPC_URL` from env/config.
  - `HELIOS_CHECKPOINT` from env/config.
- Network-specific fallback behavior:
  - If `HELIOS_NETWORK=mainnet` and consensus URL is empty, default to `HELIOS_DEFAULT_CONSENSUS_RPC_URL`.
  - If `HELIOS_NETWORK=local` and consensus URL is empty, reuse execution RPC URL.
- Startup requirements:
  - `HELIOS_EXECUTION_RPC_URL` must be set/derivable (`nixfied/.framework/helios/lifecycle.nix:131`).
  - For non-local network, `HELIOS_CONSENSUS_RPC_URL` is required (`nixfied/.framework/helios/lifecycle.nix:224`).
  - For non-local network with missing checkpoint, startup derives checkpoint from consensus API endpoints (`nixfied/.framework/helios/lifecycle.nix:140`).
- Helios command invocation:
  - Starts `${helios}/bin/helios ethereum ...` with `--network`, `--rpc-port`, `--execution-rpc`, optional `--consensus-rpc`, optional `--checkpoint` (`nixfied/.framework/helios/lifecycle.nix:231`).

Readiness tunables:

- `HELIOS_READY_TIMEOUT_SECS` default `300`
- `HELIOS_READY_INTERVAL_SECS` default `1`
- Defined/documented in `nixfied/.framework/helios/default.nix:91` and used in `nixfied/.framework/helios/lifecycle.nix:439`.

## Environment Variables Used

Primary Helios variables:

- `HELIOS_NETWORK`
- `HELIOS_EXECUTION_RPC_URL`
- `HELIOS_CONSENSUS_RPC_URL`
- `HELIOS_CHECKPOINT`
- `HELIOS_READY_TIMEOUT_SECS`
- `HELIOS_READY_INTERVAL_SECS`

Developer defaults/examples:

- `.env.example` sets `HELIOS_NETWORK=mainnet`, `HELIOS_EXECUTION_RPC_URL=https://eth.drpc.org`, and documents optional consensus/checkpoint (`.env.example:5`).
- `.env` in this repo currently mirrors mainnet-oriented values (`.env:2`).

## Dev and CI Overrides

### `mfm::portfolio::snapshot` app (`nixfied/project/dev.nix`)

- Requires `HELIOS_NETWORK=mainnet` (`nixfied/project/dev.nix:331`).
- Defaults execution RPC to `https://eth.drpc.org` if unset (`nixfied/project/dev.nix:335`).
- Starts/reuses Helios and checks readiness with:
  - `run_hook SVC_HELIOS_READY`, or
  - RPC probe (`eth_blockNumber`) when globally reused (`nixfied/project/dev.nix:447`).
- Uses Helios local RPC via source-id routing (`MFM_EVM_RPC_SOURCES_JSON`, preferred order, and
  `MFM_EVM_RPC_SOURCE_ID=helios_local`), with `MFM_EVM_RPC_URL` also set for legacy compatibility.

### CI step: parity Helios smoke

`nixfied/project/ci/scripts/steps/parity-evm-helios-smoke.nix`:

- `HELIOS_NETWORK=local`
- `HELIOS_EXECUTION_RPC_URL=http://127.0.0.1:$RETHHTTP_PORT`
- `HELIOS_CONSENSUS_RPC_URL=http://127.0.0.1:$RETHHTTP_PORT`
- `HELIOS_READY_TIMEOUT_SECS` defaulted to `120`

### CI step: mainnet portfolio snapshot (Helios)

`nixfied/project/ci/scripts/steps/mainnet-portfolio-snapshot-helios.nix`:

- `HELIOS_NETWORK=mainnet`
- `HELIOS_EXECUTION_RPC_URL` defaults to `https://eth.drpc.org`
- `HELIOS_CONSENSUS_RPC_URL` defaults to `https://lodestar-mainnet.chainsafe.io`
- `HELIOS_READY_TIMEOUT_SECS` defaults to `900`
