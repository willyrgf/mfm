# mfm-docs

Umbrella documentation entry point for the MFM workspace.

`docs.rs` publishes crates one at a time and does not provide a workspace landing page. This crate fills that gap by acting as the top-level navigation page for the published MFM surface.

Only crates that are already live on docs.rs are linked below. Remaining workspace crates stay listed by path until their publish window completes. This README is maintained manually while the public API settles.

Use this page to jump between crate families:

- engine and SDK
- core primitives
- shared states
- ops
- adapters
- storages
- collectors
- signers
- transports
- binaries and tooling

The architecture and runtime contracts live in the repository docs:

- `docs/architecture.md`
- `docs/design.md`

## Core Primitives

| Package | Role | docs.rs | Workspace Path |
| --- | --- | --- | --- |
| `mfm-adapter-contracts` | Neutral adapter contract identities and binding descriptors. | unknown | `crates/adapter-contracts` |
| `mfm-artifact-capabilities` | Artifact read capability contracts for content-addressed inputs. | unknown | `crates/artifact-capabilities` |
| `mfm-authored-config` | Shared JSON/TOML authored-config ingress helpers. | unknown | `crates/authored-config` |
| `mfm-canonical` | Canonical JSON bytes and typed content digest primitives. | unknown | `crates/kernel/canonical` |
| `mfm-capabilities` | Typed capability descriptors and role-checked capability sets. | unknown | `crates/kernel/capabilities` |
| `mfm-certify` | Typed execution-spec certification contracts. | unknown | `crates/kernel/certify` |
| `mfm-effects` | Framework-owned typed effect markers. | unknown | `crates/kernel/effects` |
| `mfm-events` | Typed kernel event schemas and event identity contracts. | unknown | `crates/kernel/events` |
| `mfm-evm-capabilities` | Reusable EVM capability contracts, requests, responses, and source evidence. | unknown | `crates/evm-capabilities` |
| `mfm-evm-contract-config` | Typed EVM contract lifecycle config schemas. | unknown | `crates/evm-contract-config` |
| `mfm-evm-contract-model` | Typed EVM contract lifecycle values and public outputs. | unknown | `crates/evm-contract-model` |
| `mfm-evm-core` | EVM ABI, encoding, hex, and transaction support types. | unknown | `crates/evm-core` |
| `mfm-ids` | Strong typed identity primitives for the typed kernel. | unknown | `crates/kernel/ids` |
| `mfm-portfolio-config` | Shared portfolio snapshot config pipeline. | unknown | `crates/portfolio-config` |
| `mfm-portfolio-model` | Pure canonical portfolio, symbol, and wallet models. | pending | `crates/portfolio/model` |
| `mfm-program` | Typed state-program authoring API and lowering evidence. | unknown | `crates/kernel/program` |
| `mfm-program-derive` | Proc-macro derives for typed kernel value and program contracts. | unknown | `crates/kernel/program-derive` |
| `mfm-replay` | Typed replay brokers and verifier contracts. | unknown | `crates/kernel/replay` |
| `mfm-runtime` | Certified serial typed scheduler and erased runner boundary. | unknown | `crates/kernel/runtime` |
| `mfm-spec` | Certified typed execution-spec data model. | unknown | `crates/kernel/spec` |
| `mfm-store` | Typed kernel commit contract and projection interfaces. | unknown | `crates/kernel/store` |
| `mfm-values` | Typed persisted value, config, and public-output descriptors. | unknown | `crates/kernel/values` |
| `mfm_core` | Security-sensitive keystore, config, and primitives. | unknown | `crates/core` |

## Shared States

| Package | Role | docs.rs | Workspace Path |
| --- | --- | --- | --- |
| `mfm-state-evm-contracts` | Reusable EVM contract lifecycle state contracts and replay semantics. | unknown | `crates/states/evm-contracts` |
| `mfm-state-portfolio` | Reusable portfolio-domain runtime states and adapters for canonical snapshots. | unknown | `crates/states/portfolio` |

## Ops

Ops stay thin and focus on graph composition and config validation. Most are still repo-local today; publish them after the shared-state crates they depend on.

| Package | Role | Workspace Path |
| --- | --- | --- |
| `mfm-op-evm-contract-lifecycle` | Contract lifecycle planner for deploy, configure, and validate phases. | `crates/ops/evm-contract-lifecycle-op` |
| `mfm-op-portfolio-tracker` | Portfolio tracking planner. | `crates/ops/portfolio-tracker-op` |
| `mfm-op-proof` | Certified typed proof workflow operation. | `crates/ops/proof-op` |

## Adapters

| Package | Role | Workspace Path |
| --- | --- | --- |
| `mfm-adapters-evm-contracts` | Adapter binding from EVM contract lifecycle state intent to runtime capabilities. | `crates/adapters/evm-contracts` |
| `mfm-adapters-portfolio` | Portfolio adapter binding from typed state intent to EVM read capabilities. | `crates/adapters/portfolio` |

## Storages

| Package | Role | Workspace Path |
| --- | --- | --- |
| `mfm-artifact-store-fs` | Filesystem artifact store. | `crates/storages/artifact-store-fs` |
| `mfm-stream-store-postgres` | PostgreSQL typed run event store. | `crates/storages/stream-store-postgres` |

## Collectors

| Package | Role | docs.rs | Workspace Path |
| --- | --- | --- | --- |
| `mfm-collectors-btc-jsonrpc-http` | Bitcoin Core JSON-RPC over HTTP client for managed Bitcoin IO. | unknown | `crates/collectors/btc-jsonrpc-http` |
| `mfm-collectors-proof` | Typed proof collector interfaces and payloads with no live IO. | unknown | `crates/collectors/proof` |

## Signers

| Package | Role | docs.rs | Workspace Path |
| --- | --- | --- | --- |
| `mfm-evm-signing` | EVM transaction signing request and signature bridge contracts. | unknown | `crates/evm-signing` |
| `mfm-signers-keystore` | Keystore-backed signer provider for generic signing requests. | unknown | `crates/signers/keystore` |
| `mfm-signing` | Generic signer references, signing requests, and provider contracts. | unknown | `crates/signing` |

## Transports

| Package | Role | Workspace Path |
| --- | --- | --- |
| `mfm-transports-evm` | Reusable EVM JSON-RPC transport and process-local source routing. | `crates/transports/evm` |
| `mfm-transports-process-exec` | Bounded child-process execution helpers for live transports. | `crates/transports/process-exec` |
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
- Public documentation publication is intentionally manual until the crate API surface stabilizes.
