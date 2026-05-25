# Operations And States Inventory

Status: living inventory of active typed operations, state contracts, runner backends, and workflow
entry points.

Source of truth:

- certified typed runner registry: `crates/app/src/lib.rs`
- typed operation planners: `crates/ops/*`
- typed state contracts: `crates/states/*` and `crates/collectors/proof`
- typed runner backends: `crates/transports/*`

Use `docs/design.md` for normative semantics and `docs/architecture.md` for placement rules.

## Current Snapshot

- Certified typed workflows: 3
- Typed operation planner crates: 3
- Typed state contract crates: 3
- Typed runner backend crates: 4
- Dynamic semantic workflow registries: 0

## Workflow Inventory

| Workflow | Planner | State contracts | Runner/backend | Primary entry points |
|---|---|---|---|---|
| proof | `crates/ops/proof-op` | `crates/collectors/proof` | `crates/transports/proof` | certified typed run start and proof conformance tests |
| portfolio tracker | `crates/ops/portfolio-tracker-op` | `crates/states/portfolio` | `crates/transports/portfolio` | `mfm portfolio snapshot`, `POST /v1/portfolio/snapshot`, certified typed run start |
| EVM deploy/configure/validate | `crates/ops/evm-deploy-configure-validate-op` | `crates/states/evm-dcv` | `crates/transports/evm-dcv` | certified typed run start and parity fixtures |

Typed run start surfaces accept certified typed execution specs. Workflow-specific commands may
compile authored JSON/TOML into a certified spec before start, but they still submit through the
typed runtime/store path.

## Typed Operation Planners

| Crate | Certified operation | Purpose |
|---|---|---|
| `crates/ops/proof-op` | proof workflow | Builds a deterministic proof program with read fact, side-effect, and output assembly states |
| `crates/ops/portfolio-tracker-op` | portfolio tracker workflow | Builds multi-network portfolio snapshot programs with domain-keyed fanout/fanin and typed public outputs |
| `crates/ops/evm-deploy-configure-validate-op` | EVM deploy/configure/validate workflow | Builds deploy, configure, and validate lifecycle programs with typed config artifacts |

Planner crates are deterministic. They do not execute states, open runtime resources, write store
events, or render public outputs by themselves.

## Typed State Contracts

### Proof

Owner: `crates/collectors/proof`

| State | Effect | Output |
|---|---|---|
| `ProofReadFactState` | read | `ProofFact` |
| `ProofApplySideEffectState` | side effect | `ProofSideEffectResult` |
| `ProofAssembleOutputState` | pure | `ProofOutput` |

The proof transport supplies deterministic runners and an evidence-only replay verifier.

### Portfolio

Owner: `crates/states/portfolio`

| State | Effect | Purpose |
|---|---|---|
| `PrepareSourcesState` | read | Prepares typed network/RPC source metadata |
| `ResolveSubjectsState` | pure | Resolves configured subjects into typed subject sets |
| `PinViewsState` | read | Pins typed chain/view material for downstream reads |
| `ResolveValuationsState` | pure | Resolves valuation routes and quote material |
| `ObserveBatchState` | read | Reads configured observations through typed portfolio backends |
| `MergeObservationsState` | pure | Merges domain-keyed observation batches |
| `AssembleSnapshotState` | pure | Produces the canonical portfolio snapshot |
| `PublishSnapshotState` | pure | Produces a typed published snapshot value |
| `ProjectReportState` | pure | Produces the typed public report |

Portfolio fanout/fanin uses stable domain keys and non-empty observation batches so duplicate or
empty semantic collections fail before runtime execution.

### EVM Deploy/Configure/Validate

Owner: `crates/states/evm-dcv`

| State | Effect | Purpose |
|---|---|---|
| `DeployContractState` | side effect | Deploys a typed contract artifact and produces `DeployedContract` |
| `ConfigureContractState` | side effect | Applies post-deploy configuration and produces `ConfiguredContract` |
| `ValidateContractState` | read | Performs typed validation reads and produces `ValidationReport` |

Deploy/configure states keep raw signed transaction material below typed semantic values. Protected
transaction bytes may be staged as managed artifacts, but they cannot become public typed values.

## Runner And Transport Inventory

| Crate | Role |
|---|---|
| `crates/transports/proof` | deterministic proof runners and proof replay verifier |
| `crates/transports/portfolio` | portfolio typed runners and RPC/artifact backend glue |
| `crates/transports/evm-dcv` | EVM lifecycle side-effect runners, validation read runners, and replay verifier |
| `crates/transports/process-exec` | bounded process execution helper for typed backends |

Runner crates bind certified descriptors to executable code. Replay-capable runners must verify
recorded evidence and must not use live external systems during replay.

## Storage Inventory

| Crate | Role |
|---|---|
| `crates/storages/stream-store-postgres` | durable typed run-event store and projections |
| `crates/storages/artifact-store-fs` | local typed artifact store |

All certified run submission, resume, replay, public-output, and retention semantics go through
`mfm-store` and typed artifact evidence.

## Assembly And Public Surfaces

| Surface | Role |
|---|---|
| `crates/app` | typed app assembly and start/resume/replay/public-output services |
| `bin/cli` | command-line typed run and workflow surface |
| `bin/rest-api` | HTTP typed run and workflow surface |

Keystore CLI commands are direct security-sensitive utility commands over `mfm_core` and do not
submit workflow runs.

## Update Policy

Update this document in the same change when any of these change:

- a typed workflow is added, removed, or renamed
- a state kind/version or public state contract changes
- a workflow gains or loses a CLI/REST entry point
- a runner backend or replay verifier is added or removed
- a typed storage implementation becomes certified or is removed
- app assembly starts linking a new production typed runner
