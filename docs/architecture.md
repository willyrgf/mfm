# Architecture

This is the short contributor map for the typed-core codebase. For normative runtime semantics, see
`docs/design.md`.

## One Sentence

MFM executes certified typed state programs: ops plan typed specs, runtime executes registered typed
states, `mfm-store` commits typed events, and CLI/REST assemble and render without owning workflow
semantics.

## End-To-End Flow

```text
typed or authored input
  -> operation crate builds typed program draft
  -> mfm-certify emits certified typed execution spec
  -> app persists spec/config/seed artifacts
  -> runtime lowers certified spec through registered runners
  -> store appends typed events and projections
  -> replay/resume/public-output read from certified spec + run stream
```

The certified typed execution spec is the runtime contract. Any runner plan is an implementation
detail that must be derivable from that spec.

## Authority Contract

The typed boundary separates transport data from authority:

- parsed typed spec JSON is data only
- `HashedSpecEnvelope` is a hash-only envelope only
- persisted `CertifiedSpecCertificate` bytes are evidence only until verified
- `CertifiedTypedSpec` is the non-forgeable authority returned by `mfm-certify`
- `CertifiedRuntimeSpec` is runtime authority derived only from `CertifiedTypedSpec`
- erased runner plans are implementation artifacts
- rendered public-output JSON is an output/cache surface only

Start, resume, replay, and public-output rendering must verify stored spec/certificate artifacts
against the production registry and compare stream evidence before constructing runtime, replay, or
render authority. Hash matches, certificate bytes, stored summaries, source scans, and naming
conventions are not semantic authority.

## Layers

| Layer | Crates | Owns |
|---|---|---|
| Transport surfaces | `bin/cli`, `bin/rest-api` | Input decoding, response envelopes, command/API routing |
| App assembly | `crates/app` | Store/artifact/runner/capability assembly and typed run services |
| Operations | `crates/ops/*` | Deterministic typed program planning |
| States | `crates/states/*`, typed domain state crates | Typed state contracts and state-owned deterministic behavior |
| Transports | `crates/transports/*` | Certified runner bindings, live/replay capability backends |
| Storages | `crates/storages/*` | Typed run-event and typed artifact persistence |
| Kernel | `crates/kernel/*` | Typed ids, values, programs, specs, certification, events, store, runtime, replay |
| Domain models | `crates/*-model`, `crates/*-config`, `crates/core`, `crates/evm-core` | Pure domain types, validation, encoding, and security-sensitive primitives |

## Kernel Boundary

Kernel crates are domain-free and point inward only through the kernel dependency DAG:

```text
ids -> canonical -> values -> effects/capabilities
  -> program/spec -> certify/events/store -> runtime/replay
```

Kernel crates must not depend on app, binaries, domain models, typed states, operations, transports,
or storage implementations.

## Planning Boundary

Operation crates are planners. They may:

- parse and validate typed planning config
- call other typed operation builders
- create seeds, scopes, state nodes, bridge nodes, and public-output bindings
- attach stable domain-key and lineage evidence
- return a typed program draft or certified spec helper

Operation crates must not:

- execute runtime behavior
- open files, network connections, processes, or signers for semantic work
- write store events
- decide resume/replay behavior
- hide state behavior in app/bin glue

If a change needs new executable behavior, put that behavior in a typed state or typed runner, not
in an operation.

## State Boundary

State crates own typed state contracts:

- `StateSpec` metadata
- config/input/output value types
- effect class
- required capability set
- side-effect contract when applicable
- deterministic validation and transformation logic

State crates must not depend on runtime, store implementations, app, or binaries. Side-effecting or
external observation behavior is reached through typed capabilities supplied by runtime/app
assembly, not through ambient access.

## Transport Boundary

Transport crates bind certified state descriptors to runtime runners and live/replay capability
backends. They may persist typed facts or artifacts through boundaries supplied by app/runtime, but
they do not become semantic authority by themselves.

Active typed transports:

- `crates/transports/proof`
- `crates/transports/portfolio`
- `crates/transports/evm-dcv`
- `crates/transports/process-exec` as a bounded process helper for typed backends

Replay implementations must answer from recorded evidence only.

## Store Boundary

`mfm-store` defines the commit contract. Implementations accept typed commit payloads and
preconditions; they construct envelopes, assign sequence/ordinal/event identities, enforce logical
keys, and maintain projections.

Active storage implementations:

- `crates/storages/stream-store-postgres`: durable typed run-event store
- `crates/storages/artifact-store-fs`: local typed artifact store

Projection data is derived from the run stream and is never the sole authority for semantic resume,
replay, retention, public output, or side-effect status.

## App And Binary Boundary

`crates/app` wires:

- typed runner registries
- typed capability backends
- typed run-event stores
- typed artifact stores
- certified start/resume/replay services
- typed public-output rendering

`bin/cli` and `bin/rest-api` stay transport-only. They may expose stable JSON/text/HTTP surfaces,
but all semantic work flows through typed operations, app services, runtime, store, and replay.
Generic start accepts certified bundles. Domain start routes may accept domain inputs, but they must
certify typed specs internally before `RunStarted`.

## Active Workflow Ports

| Workflow | Planner | State contracts | Runner/backend |
|---|---|---|---|
| proof | `crates/ops/proof-op` | `crates/collectors/proof` | `crates/transports/proof` |
| portfolio tracker | `crates/ops/portfolio-tracker-op` | `crates/states/portfolio` | `crates/transports/portfolio` |
| EVM deploy/configure/validate | `crates/ops/evm-deploy-configure-validate-op` | `crates/states/evm-dcv` | `crates/transports/evm-dcv` |

Keystore CLI commands currently call `mfm_core` keystore primitives directly and do not submit
workflow runs.

## Release Tooling

`publish-docs` is separate release tooling for crate documentation publication. `plan` produces
the reviewable publication plan, while `apply` and the default command can run `cargo publish` and
follow-up documentation sync work; it is not part of the typed workflow runtime surface.

## Placement Guide

- New typed value/config/output type: domain model/config crate, deriving typed descriptor traits.
- New reusable state behavior: a `crates/states/*` crate.
- New workflow topology: an operation crate.
- New live or replay external backend: a transport crate.
- New store implementation: a storage crate implementing typed store/artifact contracts.
- New start/resume/replay assembly: `crates/app`.
- New command/API shape: `bin/cli` or `bin/rest-api`, backed by app services.
- New kernel semantic primitive: a `crates/kernel/*` crate, with no domain dependencies.

## Review Checklist

Before merging a change, verify:

- typed specs remain the only runtime contract
- new public values/configs use typed descriptors and no floats/secrets
- side effects have typed intent, idempotency, receipt/recovery, and one mutation authority
- replay paths cannot construct live capabilities
- resume validates stored stream evidence against the certified spec
- app/bin changes do not embed planner or state behavior
- docs and tests are updated in the same change
- source scans or CI summary keys are not used as typed-core proof

## CI Ownership

Typed-core guarantees are owned by Rust type/API boundaries, compile-fail fixtures, cargo metadata
tests, schema golden tests, and production-path integration tests. The former typed-core
source-scan gates and summary-key validators have been deleted from Nixfied CI and are historical
only.

## Companion Docs

- `docs/design.md`: normative design contract
- `docs/ops-and-states.md`: active typed workflow and state inventory
- `bin/cli/README.md`: CLI command and JSON output contract
- `bin/rest-api/README.md`: REST contract
- `RFC_TYPED_CORE_PROPOSAL_1.md`: historical RFC for the rewrite
