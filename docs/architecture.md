# Architecture

Status: contributor architecture guide.

For normative runtime, store, replay, and authority semantics, see `docs/design.md`. This document
defines the placement taxonomy and boundary rules contributors must use when adding or changing
MFM crates, states, operations, adapters, transports, signers, configs, CLI commands, REST routes,
and tests.

## One Sentence

MFM runs event-sourced typed state-machine workflows: operations plan certified typed specs, states
own reusable domain semantics, adapters bind state intent to explicit capabilities, transports and
signers implement reusable platform primitives, runtime schedules from certified authority, store
commits append-only typed events, and CLI/REST remain transport-only surfaces.

## Core Runtime Shape

```text
typed or authored input
  -> operation crate builds typed program draft
  -> mfm-certify emits certified typed execution spec
  -> app verifies certified bundle and assembles launch material
  -> runtime rebuilds verified history from the append-only run stream
  -> deterministic frontier scheduler selects one certified node or terminal decision
  -> sealed runner invocation produces typed intent, staged artifacts, or sealed handles
  -> runtime commit planner builds PreparedTypedCommit
  -> store atomically admits artifact evidence and appends typed events
  -> replay/resume/public-output read from certified spec plus run stream
```

The certified typed execution spec is the runtime contract. Runner plans, route names, command
names, source scans, CI summary keys, rendered JSON, and projection rows are not semantic authority.

## Authority Contract

The typed boundary separates data, evidence, authority, and implementation artifacts:

- parsed typed spec JSON is data only
- `HashedSpecEnvelope` is a hash-only envelope only
- persisted `CertifiedSpecCertificate` bytes are evidence only until verified
- `CertifiedTypedSpec` is the non-forgeable authority returned by `mfm-certify`
- `CertifiedRuntimeSpec` is runtime authority derived only from `CertifiedTypedSpec`
- erased runner plans are implementation artifacts
- rendered public-output JSON is an output/cache surface only

Start, resume, replay, and public-output rendering must verify stored spec/certificate artifacts
against the production registry and compare stream evidence before constructing runtime, replay, or
render authority.

## Taxonomy

Every new unit must declare which category it belongs to before it gets a crate, schema namespace,
capability, public type, CLI command, REST route, or test fixture.

| Category | Owns | Does Not Own |
|---|---|---|
| Platform primitive | Reusable infrastructure such as signing, protocol clients, source routing, artifact access, process execution | Workflow topology or domain-specific semantics |
| Domain model/config | Pure domain types, validation, canonical config, schema descriptors | Runtime IO, signer resolution, transport clients |
| State | Reusable executable domain semantics and typed state contracts | Ambient IO, app/store authority, protocol implementation |
| Operation | Deterministic graph topology and config lowering | Runtime execution, IO, signers, transports, replay |
| Adapter | Runner binding from state intent to capabilities and evidence recording | Generic protocol clients, signer provider internals, operation topology |
| Transport | Reusable protocol implementation and live/replay capability backend | Workflow recipes, signer material, domain topology |
| Signer | Generic signer refs, signing requests/results, signing provider implementations | Workflow recipes, raw signed transaction persistence |
| Storage | Typed run-event or artifact persistence implementation | Domain semantics, scheduler behavior |
| App assembly | Registry/store/artifact/capability wiring and typed run services | Workflow planning or state behavior |
| Binary/API | Input decoding, routing, response envelopes | Domain semantics, runtime authority, direct state execution |

If a unit does not fit one category cleanly, the design is not ready.

## Boundary Contract

### Operation

Operations are deterministic planning only.

Operations may:

- parse and validate typed planning config
- lower authored config into canonical typed config
- call other typed operation builders
- create seeds, scopes, state nodes, bridge nodes, and public-output bindings
- attach stable domain-key and lineage evidence
- return a typed program draft or certified spec helper

Operations must not:

- execute runtime behavior
- open files, network connections, processes, RPC clients, or signers for semantic work
- read environment variables for runtime routing
- sign payloads
- submit transactions
- poll receipts
- write store events
- decide resume or replay behavior
- own reusable protocol APIs
- own generic signer APIs
- hide state behavior in app or binary glue

Workflow topology names may appear at this layer. They must not leak into lower layers unless the
name is also durable domain language.

### State

States own reusable domain lifecycle behavior and typed contracts.

States may:

- define `StateSpec` metadata
- define config, input, output, public-output, and artifact value types
- declare effect class and required capabilities
- define side-effect contract when applicable
- validate domain config shape
- construct deterministic domain intent
- own deterministic transformation and validation semantics
- define replay-checkable request/response semantics

States must not:

- build HTTP or JSON-RPC clients
- parse runtime source environment variables
- open files or processes
- open keystores
- read passwords
- sign payloads
- submit raw transactions
- persist runtime evidence directly
- depend on runtime, store implementations, app, binaries, transport implementations, or signer
  implementations

Side-effecting or external observation behavior is reached through typed capabilities supplied by
runtime/app assembly. State code must not create its own live network, filesystem, clock, process,
or signer access when that access is part of semantic execution.

### Adapter

Adapters connect state-owned domain intent to runtime capabilities.

Adapters may:

- bind certified state descriptors to executable runners
- materialize typed inputs through runtime-provided surfaces
- call generic transports
- call generic signer providers
- encode submit-time raw transaction bytes transiently
- stage typed artifacts and sealed handles
- record side-effect evidence through runtime/store APIs
- implement domain replay verifiers from recorded evidence

Adapters must not:

- define generic protocol clients under workflow names
- own signer material resolution directly
- bypass runtime/store commit authority
- place semantic behavior outside state-owned functions
- depend on workflow operation crates for runtime behavior
- persist secrets, raw signing material, or raw signed transaction bytes

If adapter code currently lives under `crates/transports/*`, it still must obey adapter rules. A
workflow-specific runner crate is not a license to own generic transport or signer behavior.

### Transport

Transports implement reusable protocols and live/replay capability backends.

Transports may:

- parse runtime-only source configuration
- select protocol sources by non-secret runtime refs
- redact endpoints and authorization material
- execute protocol calls
- expose protocol traits and request/response types
- support multiple adapters, states, and workflows
- answer replay requests from recorded evidence only when implementing replay backends

Transports must not:

- use workflow recipe names
- depend on workflow operation crates
- know about workflow topology
- open keystores or own signer behavior
- read password files
- persist typed semantic events by themselves
- leak RPC URLs, authorization headers, or local routing details into typed semantic surfaces

If two workflows can use the same protocol behavior, that behavior belongs in a shared transport
before either workflow lands.

### Signer

Signers provide generic key material and signature capabilities.

Signer crates may:

- define signer references
- define signing algorithm and domain identifiers
- define signing request/result traits
- implement MFM keystore-backed signing
- map runtime signer ids to keystore entries, hardware signers, remote signers, or future wallets
- verify expected public identities
- redact all secret-bearing details from errors

Signer crates must not:

- depend on workflow operation crates
- depend on domain lifecycle state crates
- know about CLI or REST request shapes
- persist private keys, mnemonics, passwords, password paths, keystore paths, signature scalars, or
  raw signed transaction bytes in typed semantic surfaces

Raw signed transactions are bearer mutation material. They remain transient submit-time bytes below
the typed semantic boundary.

### Configuration

Typed config contains semantic intent and non-secret references only.

Typed config may contain:

- domain intent
- content-addressed artifact refs or inline non-secret artifacts
- network id
- source id when source selection is semantic
- expected chain id or equivalent protocol identity
- signer id
- expected public address or public identity
- receipt, retry, or validation policy
- assertion definitions

Typed config must not contain:

- private keys
- mnemonics
- passwords
- password file paths
- keystore file paths
- environment variable names that resolve secret-bearing paths
- RPC URLs
- RPC authorization headers
- raw signed transaction bytes
- signature material

Runtime process configuration maps non-secret refs to concrete local resources. That mapping must
not become typed workflow config, event payload, artifact payload, public output, fixture, or replay
input.

## Architecture Doctrine

### Rule 1: One Stable Abstraction Per Crate

A crate must own one durable abstraction. It must not own a parity slice.

Good crate reasons:

- reusable protocol transport
- reusable signer provider
- reusable domain state family
- deterministic workflow topology
- pure domain model
- authored/canonical config pipeline
- storage implementation
- runtime/kernel primitive

Bad crate reasons:

- "this was the scope of the parity test"
- "these functions were implemented together"
- "the CLI command needs a place to put code"
- "the recipe name is convenient"
- "this is experimental so naming does not matter"

### Rule 2: Workflow Names Stop At Operations

Workflow topology names may appear in:

- operation crate names
- operation type names
- operation tests
- public commands or routes only when the public workflow name is durable domain language

Workflow topology names must not appear in:

- generic transport crates
- signer crates
- protocol clients
- capability names
- lower-layer domain model namespaces
- state crate names when the states are independently reusable
- public schema namespaces when the behavior has broader domain meaning

### Rule 3: Capability Names Describe Authority

Capability names must describe the authority being granted, not the workflow requesting it.

Allowed:

- `mfm.evm.rpc.read`
- `mfm.evm.transaction.submit`
- `mfm.signing.sign`
- `mfm.artifact.read`

Disallowed:

- feature-specific names for generic EVM RPC reads
- workflow-specific names for generic signer access
- recipe-specific names for transaction submission

The effect/capability role count can be correct while the capability taxonomy is wrong. Reviewers
must check both.

### Rule 4: Public APIs Use Domain Language

Public names should describe what the user means, not how the implementation was assembled.

Allowed public names:

- contract deployment
- contract configuration
- contract validation
- contract lifecycle
- EVM transaction intent
- EVM RPC source
- signer reference
- signing provider

Disallowed public names:

- temporary acronyms
- parity fixture names
- implementation recipe names
- transport crate names exposed as route or command names

### Rule 5: Runtime Routing Is Not Semantic Config

Semantic config may refer to runtime systems by stable non-secret ids. Runtime process config maps
those ids to concrete local resources.

Keep these concepts distinct:

- `network_id`: semantic domain network label
- `source_id`: local runtime source routing key
- `expected_chain_id`: concrete chain identity returned by the network
- `control_scope`: MFM execution partition, only if truly semantic
- `signer_id`: non-secret signer reference

Local development chains must not be labeled as mainnet networks unless they are actually mainnet.

### Rule 6: Tests Must Enforce Boundaries

Tests should not merely prove the implemented shape works. They must also reject wrong shapes.

Boundary tests should include:

- cargo metadata dependency checks
- compile-fail tests for forbidden typed surfaces
- schema namespace golden tests
- redaction tests for secret-bearing values and errors
- production-path integration tests for replay and side-effect evidence
- route/command tests that assert domain-language public APIs

Source scans may be useful as guardrails, but they are not architecture proof by themselves.

## Placement Guide

- New kernel semantic primitive: `crates/kernel/*`, with no domain dependencies.
- New pure domain type or canonical config type: domain model/config crate.
- New reusable state behavior: `crates/states/*` or another clearly named domain state crate.
- New workflow topology: `crates/ops/*-op`.
- New runner binding from state intent to capabilities: adapter crate, or transport crate obeying
  adapter rules until an adapter namespace exists.
- New reusable live/replay protocol backend: `crates/transports/*`.
- New signer abstraction or provider: `crates/signing` or `crates/signers/*`.
- New store implementation: `crates/storages/*`.
- New start/resume/replay/public-output assembly: `crates/app`.
- New command/API shape: `bin/cli` or `bin/rest-api`, backed by app services.

## Dependency Rules

Kernel crates point inward only through the kernel dependency DAG:

```text
ids -> canonical -> values -> effects/capabilities
  -> program/spec -> certify/events/store -> runtime/replay
```

Additional dependency rules:

- kernel crates must not depend on app, binaries, domain models, states, operations, transports,
  signers, or storage implementations
- states must not depend on runtime, store implementations, app, binaries, transport
  implementations, signer implementations, or operation crates
- operations may depend on typed states and domain config/model crates, but not on transports,
  signer implementations, app, binaries, runtime scheduling, or storage implementations
- transports may depend on protocol/domain support crates and kernel contracts, but not on workflow
  operation crates
- signer providers may depend on security-sensitive core primitives, but not on workflow operation
  or state crates
- binaries may depend on app and operation/config crates for input compilation, but must not own
  workflow semantics

## Store Boundary

`mfm-store` defines the production commit contract. Implementations accept only
`PreparedTypedCommit` for execution mutation. Each prepared commit carries typed payloads and
artifact evidence to admit atomically with those payloads.

Stores own:

- event envelopes
- sequence numbers
- ordinals
- event ids
- logical keys
- commit preconditions
- projections

Projection data is derived from the run stream and is never the sole authority for semantic resume,
replay, retention, public output, or side-effect status.

Synthetic store mutation is reserved for explicitly named non-execution test, migration, repair,
corruption, or low-level storage contract fixtures.

## App And Binary Boundary

`crates/app` wires:

- typed runner registries
- typed capability backends
- typed run-event stores
- typed artifact stores
- certified start/resume/replay services
- typed public-output rendering

`crates/app` must not own workflow planning or state behavior.

`bin/cli` and `bin/rest-api` may:

- decode JSON/TOML/user input
- build typed configs and certified specs through operation crates
- select stores, artifacts, runners, and capabilities through app services
- start, resume, replay, inspect, and render typed runs
- preserve stable response envelopes

`bin/cli` and `bin/rest-api` must not:

- plan workflow semantics directly
- execute state behavior directly
- bypass typed certification
- infer public outputs from untyped snapshots
- migrate uncertified historical runs into certified typed runs

## Public Naming Rules

Public CLI commands, REST routes, request kinds, schema ids, error codes, executable identities, and
public-output schema ids are durable contracts.

Before adding or renaming a public surface, verify:

- the name is domain language, not implementation scaffolding
- the name would still make sense if the current operation topology changed
- the name does not expose an adapter, transport, crate, parity fixture, or temporary acronym
- the schema namespace is stable enough to persist in artifacts and run streams
- tests assert the public domain name and reject stale recipe names

## Review Checklist

Before merging a change, verify:

- the new unit has exactly one taxonomy category
- the crate name describes a durable abstraction
- typed specs remain the only runtime contract
- new public values/configs use typed descriptors and no floats/secrets
- side effects have typed intent, idempotency, receipt/recovery, one mutation authority, and no
  retained signed raw transactions
- replay paths cannot construct live capabilities
- resume validates stored stream evidence against the certified spec
- app/bin changes do not embed planner or state behavior
- generic protocol behavior is not implemented inside workflow-specific crates
- signer behavior is not implemented inside workflow-specific crates
- typed config contains only semantic intent and non-secret references
- public CLI/REST/schema names use durable domain language
- tests enforce boundaries, not only current implementation shape
- docs and tests are updated in the same change

## CI Ownership

Architecture guarantees should be enforced by:

- Rust type/API boundaries
- private constructors and sealed traits where appropriate
- compile-fail fixtures
- cargo metadata dependency checks
- schema golden tests
- redaction tests
- production-path integration tests

Required metadata checks should assert:

- `crates/states/*` do not depend on transports, signer implementations, app, binaries, or storage
  implementations
- `crates/ops/*` do not depend on transports, signer implementations, app, binaries, runtime
  scheduling, or storage implementations
- generic transports do not depend on workflow operation crates
- signer providers do not depend on workflow operation or state crates
- public schema namespaces do not use temporary recipe names

## Release Tooling

`publish-docs` is separate release tooling for crate documentation publication. It is not part of
the typed workflow runtime surface and must not be used as architectural proof.

## Companion Docs

- `docs/design.md`: authoritative runtime, store, replay, and secret-handling contract
- `docs/code-quality.md`: mandatory quality policy for code, test, documentation, build, and
  workflow changes
- `PROBLEM_ARCH_DCV.md`: architecture correction case study for recipe-boundary failure
- `bin/cli/README.md`: CLI command and JSON output contract
- `bin/rest-api/README.md`: REST contract
- `RFC_TYPED_CORE_PROPOSAL_1.md`: historical RFC for the typed-core rewrite
