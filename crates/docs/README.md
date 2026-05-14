# mfm-docs

Umbrella documentation entry point for the MFM workspace.

`docs.rs` publishes crates one at a time and does not provide a workspace landing page. This crate fills that gap by acting as the top-level navigation page for the published MFM surface.

Only crates that are already live on docs.rs are linked below. Remaining workspace crates stay listed by path until their publish window completes.

`crates/docs/catalog.toml` is authoritative for this README; `sync-umbrella` rewrites the package tables from that catalog.

Use this page to jump between crate families:

- engine and SDK
- core primitives
- shared states
- ops
- storages
- collectors
- transports
- binaries and tooling

The live runtime inventory still lives in the repository docs:

- `docs/ops-and-states.md`
- `docs/architecture.md`
- `docs/design.md`

## Engine And SDK

| Package | Role | docs.rs | Workspace Path |
| --- | --- | --- | --- |
| `mfm-machine` | State-machine runtime, execution plans, events, and recovery contracts. | <https://docs.rs/mfm-machine> | `crates/machine` |
| `mfm-machine-derive` | Reserved proc-macro boundary for future machine/state helpers. | <https://docs.rs/mfm-machine-derive> | `crates/machine-derive` |
| `mfm-machine-test-support` | Contract tests and test observability helpers. | <https://docs.rs/mfm-machine-test-support> | `crates/machine-test-support` |
| `mfm-sdk` | Run launch, resume, and registry helpers. | <https://docs.rs/mfm-sdk> | `crates/sdk` |

## Core Primitives

| Package | Role | docs.rs | Workspace Path |
| --- | --- | --- | --- |
| `mfm-authored-config` | Shared JSON/TOML authored-config ingress helpers. | pending | `crates/authored-config` |
| `mfm-evm-core` | EVM ABI, encoding, hex, and transaction support types. | <https://docs.rs/mfm-evm-core> | `crates/evm-core` |
| `mfm-evm-deploy-configure-validate-config` | Shared deploy/configure/validate config pipeline. | pending | `crates/evm-deploy-configure-validate-config` |
| `mfm-portfolio-config` | Shared portfolio snapshot config pipeline. | pending | `crates/portfolio-config` |
| `mfm_core` | Security-sensitive keystore, config, and primitives. | pending | `crates/core` |

## Shared States

| Package | Role | docs.rs | Workspace Path |
| --- | --- | --- | --- |
| `mfm-evm-runtime` | Shared EVM read and write runtime states. | pending | `crates/evm-runtime` |
| `mfm-state-aave-v3` | Shared Aave V3 portfolio-position types and runtime adapters. | pending | `crates/states/aave-v3` |
| `mfm-state-common` | Cross-domain reusable execution states. | pending | `crates/states/common` |
| `mfm-state-keystore` | Shared keystore administration and local signing states. | pending | `crates/states/keystore` |
| `mfm-state-portfolio` | Reusable portfolio-domain schema and validation for canonical snapshots. | pending | `crates/states/portfolio` |
| `mfm-state-symbol` | Reusable symbol-domain schema and validation for canonical snapshots. | pending | `crates/states/symbol` |
| `mfm-state-wallet` | Reusable wallet-domain schema and validation for canonical snapshots. | pending | `crates/states/wallet` |

## Ops

Ops stay thin and focus on graph composition and config validation. Most are still repo-local today; publish them after the shared-state crates they depend on.

| Package | Role | Workspace Path |
| --- | --- | --- |
| `mfm-op-evm-deploy-configure-validate` | EVM deploy/configure/validate planner. | `crates/ops/evm-deploy-configure-validate-op` |
| `mfm-op-evm-read` | EVM read planner. | `crates/ops/evm-read-op` |
| `mfm-op-evm-write` | EVM write planner. | `crates/ops/evm-write-op` |
| `mfm-op-keystore-admin` | Keystore admin planner. | `crates/ops/keystore-admin-op` |
| `mfm-op-keystore-tx` | Keystore transaction planner. | `crates/ops/keystore-tx-op` |
| `mfm-op-nix-app` | Nix app execution planner. | `crates/ops/nix-app-op` |
| `mfm-op-portfolio-tracker` | Portfolio tracking planner. | `crates/ops/portfolio-tracker-op` |
| `mfm-op-proof` | Proof-generation planner. | `crates/ops/proof-op` |

## Storages

| Package | Role | Workspace Path |
| --- | --- | --- |
| `mfm-artifact-store-fs` | Filesystem artifact store. | `crates/storages/artifact-store-fs` |
| `mfm-artifact-store-s3` | S3-backed artifact store. | `crates/storages/artifact-store-s3` |
| `mfm-artifact-store-secret` | Secret-wrapping artifact store. | `crates/storages/artifact-store-secret` |
| `mfm-control-plane-postgres` | Durable RPC control-plane stream/projection storage. | `crates/storages/control-plane-postgres` |
| `mfm-stream-store-mem` | In-memory stream store. | `crates/storages/stream-store-mem` |
| `mfm-stream-store-postgres` | PostgreSQL stream store. | `crates/storages/stream-store-postgres` |

## Collectors

| Package | Role | docs.rs | Workspace Path |
| --- | --- | --- | --- |
| `mfm-collectors-btc-jsonrpc-http` | Bitcoin Core JSON-RPC over HTTP client for managed Bitcoin IO. | pending | `crates/collectors/btc-jsonrpc-http` |
| `mfm-collectors-evm` | EVM collector traits and payloads. | <https://docs.rs/mfm-collectors-evm> | `crates/collectors/evm` |
| `mfm-collectors-evm-jsonrpc-http` | HTTP JSON-RPC collector implementation for EVM. | pending | `crates/collectors/evm-jsonrpc-http` |
| `mfm-collectors-exec` | Command-execution collector interfaces. | pending | `crates/collectors/exec` |
| `mfm-collectors-local-fs` | Local filesystem collector interfaces. | pending | `crates/collectors/local-fs` |
| `mfm-collectors-local-keystore` | Local keystore collector interfaces. | pending | `crates/collectors/local-keystore` |
| `mfm-collectors-nix` | Nix evaluation collector interfaces. | pending | `crates/collectors/nix` |
| `mfm-collectors-nix-exec` | Nix execution collector implementation. | pending | `crates/collectors/nix-exec` |
| `mfm-collectors-proof` | Typed proof collector interfaces and payloads with no live IO. | pending | `crates/collectors/proof` |
| `mfm-collectors-rpc-control` | Typed `rpc.control` adapters for managed EVM RPC calls. | pending | `crates/collectors/rpc-control` |

## Transports

| Package | Role | Workspace Path |
| --- | --- | --- |
| `mfm-transports-local-evm` | Local EVM signing and transaction transport. | `crates/transports/local-evm` |
| `mfm-transports-local-fs` | Local filesystem transport helpers. | `crates/transports/local-fs` |
| `mfm-transports-local-keystore` | Local keystore-backed transport helpers. | `crates/transports/local-keystore` |
| `mfm-transports-proof` | Acceptance/demo proof live transport helpers. | `crates/transports/proof` |
| `mfm-transports-rpc-control` | Managed `rpc.control` transport over the internal EVM executor. | `crates/transports/rpc-control` |

## Binaries And Tooling

| Package | Role | Workspace Path |
| --- | --- | --- |
| `mfm` | CLI package and `mfm_cli` binary. | `bin/cli` |
| `mfm-app` | App-level registry and observability glue. | `crates/app` |
| `mfm-integration-tests` | Workspace integration-test crate. | `tests/integration` |
| `mfm-rest-api` | REST API package and `mfm_rest_api` binary. | `bin/rest-api` |

## Publishing Notes

- Publish this crate after the first published docs.rs surface is live, then add links for later crates as they land.
- Keep this page role-oriented and high-level; detailed inventories belong in the repository docs.
- The publish order is tracked in `crates/docs/publish-wave.json` today; `crates/docs/publish-wave.toml` is also accepted when the JSON file is absent.
