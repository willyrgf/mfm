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

## Core Primitives

| Package | Role | docs.rs | Workspace Path |
| --- | --- | --- | --- |
| `mfm-authored-config` | Shared JSON/TOML authored-config ingress helpers. | pending | `crates/authored-config` |
| `mfm-canonical` | Canonical JSON bytes and typed content digest primitives. | pending | `crates/kernel/canonical` |
| `mfm-capabilities` | Typed capability descriptors and role-checked capability sets. | pending | `crates/kernel/capabilities` |
| `mfm-certify` | Typed execution-spec certification contracts. | pending | `crates/kernel/certify` |
| `mfm-effects` | Framework-owned typed effect markers. | pending | `crates/kernel/effects` |
| `mfm-events` | Typed kernel event schemas and event identity contracts. | pending | `crates/kernel/events` |
| `mfm-evm-core` | EVM ABI, encoding, hex, and transaction support types. | <https://docs.rs/mfm-evm-core> | `crates/evm-core` |
| `mfm-evm-dcv-model` | Pure EVM deploy/configure/validate model and ABI preparation helpers. | pending | `crates/evm-dcv-model` |
| `mfm-evm-deploy-configure-validate-config` | Shared deploy/configure/validate config pipeline. | pending | `crates/evm-deploy-configure-validate-config` |
| `mfm-ids` | Strong typed identity primitives for the typed kernel. | pending | `crates/kernel/ids` |
| `mfm-kernel-test-support` | Shared test support for typed kernel contract fixtures. | pending | `crates/kernel/test-support` |
| `mfm-portfolio-config` | Shared portfolio snapshot config pipeline. | pending | `crates/portfolio-config` |
| `mfm-portfolio-model` | Pure canonical portfolio, symbol, and wallet models. | pending | `crates/portfolio/model` |
| `mfm-program` | Typed state-program authoring API and lowering evidence. | pending | `crates/kernel/program` |
| `mfm-program-derive` | Proc-macro derives for typed kernel value and program contracts. | pending | `crates/kernel/program-derive` |
| `mfm-replay` | Typed replay brokers and verifier contracts. | pending | `crates/kernel/replay` |
| `mfm-runtime` | Certified serial typed scheduler and erased runner boundary. | pending | `crates/kernel/runtime` |
| `mfm-spec` | Certified typed execution-spec data model. | pending | `crates/kernel/spec` |
| `mfm-store` | Typed kernel commit contract and projection interfaces. | pending | `crates/kernel/store` |
| `mfm-values` | Typed persisted value, config, and public-output descriptors. | pending | `crates/kernel/values` |
| `mfm_core` | Security-sensitive keystore, config, and primitives. | pending | `crates/core` |

## Shared States

| Package | Role | docs.rs | Workspace Path |
| --- | --- | --- | --- |
| `mfm-state-evm-dcv` | Typed EVM deploy/configure/validate lifecycle state contracts. | pending | `crates/states/evm-dcv` |
| `mfm-state-portfolio` | Reusable portfolio-domain runtime states and adapters for canonical snapshots. | pending | `crates/states/portfolio` |

## Ops

Ops stay thin and focus on graph composition and config validation. Most are still repo-local today; publish them after the shared-state crates they depend on.

| Package | Role | Workspace Path |
| --- | --- | --- |
| `mfm-op-evm-deploy-configure-validate` | EVM deploy/configure/validate planner. | `crates/ops/evm-deploy-configure-validate-op` |
| `mfm-op-portfolio-tracker` | Portfolio tracking planner. | `crates/ops/portfolio-tracker-op` |
| `mfm-op-proof` | Certified typed proof workflow operation. | `crates/ops/proof-op` |

## Storages

| Package | Role | Workspace Path |
| --- | --- | --- |
| `mfm-artifact-store-fs` | Filesystem artifact store. | `crates/storages/artifact-store-fs` |
| `mfm-stream-store-postgres` | PostgreSQL typed run event store. | `crates/storages/stream-store-postgres` |

## Collectors

| Package | Role | docs.rs | Workspace Path |
| --- | --- | --- | --- |
| `mfm-collectors-btc-jsonrpc-http` | Bitcoin Core JSON-RPC over HTTP client for managed Bitcoin IO. | pending | `crates/collectors/btc-jsonrpc-http` |
| `mfm-collectors-proof` | Typed proof collector interfaces and payloads with no live IO. | pending | `crates/collectors/proof` |

## Transports

| Package | Role | Workspace Path |
| --- | --- | --- |
| `mfm-transports-evm-dcv` | Typed EVM deploy/configure/validate workflow runners. | `crates/transports/evm-dcv` |
| `mfm-transports-process-exec` | Bounded child-process execution helpers for live transports. | `crates/transports/process-exec` |
| `mfm-transports-portfolio` | Typed portfolio workflow runners. | `crates/transports/portfolio` |
| `mfm-transports-proof` | Deterministic typed proof runners and conformance fixture. | `crates/transports/proof` |

## Binaries And Tooling

| Package | Role | Workspace Path |
| --- | --- | --- |
| `mfm` | CLI package and `mfm_cli` binary. | `bin/cli` |
| `mfm-app` | Typed application assembly for certified runtime services. | `crates/app` |
| `mfm-integration-tests` | Workspace integration-test crate. | `tests/integration` |
| `mfm-rest-api` | REST API package and `mfm_rest_api` binary. | `bin/rest-api` |

## Publishing Notes

- Publish this crate after the first published docs.rs surface is live, then add links for later crates as they land.
- Keep this page role-oriented and high-level; detailed inventories belong in the repository docs.
- The publish order is tracked in `crates/docs/publish-wave.json` today; `crates/docs/publish-wave.toml` is also accepted when the JSON file is absent.
