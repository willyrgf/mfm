# Architecture

Status: contributor architecture guide.

For normative runtime, store, replay, and authority semantics, see `docs/design.md`. This document
defines the placement taxonomy and boundary rules contributors must use when adding or changing
MFM crates, states, operations, adapters, transports, signers, configs, CLI commands, REST routes,
and tests.

## One Sentence

MFM runs event-sourced typed state-machine workflows: operations plan certified typed specs, states
own reusable domain semantics, adapters bind state intent to explicit capabilities, transports and
signers implement reusable platform primitives, runtime schedules certified saga-aware authority,
store commits append-only typed events, replay verifies from evidence only, and CLI/REST remain
transport-only surfaces.

## Core Runtime Shape

```text
setup TOML
  -> app strictly decodes, validates, canonicalizes, scans for prohibited fields
  -> app atomically upserts complete typed values by intrinsic stable target
entry-point id plus target
  -> app resolves and verifies the target's current configuration
  -> operation crate builds a concrete typed program draft
  -> mfm-certify lowers, validates, and emits certified typed execution spec
  -> app verifies persisted spec/certificate evidence and assembles launch material
  -> runtime rebuilds verified history from the append-only run stream
  -> deterministic frontier scheduler selects one certified node or terminal decision
  -> sealed runner invocation produces typed intent, staged artifacts, or sealed handles
  -> runtime commit planner builds purpose-specific PreparedCommitPlan
  -> store atomically admits artifact evidence and appends typed events
  -> replay/resume/public-output read from certified spec plus run stream
```

The certified typed execution spec is the runtime contract. Runner plans, route names, command
names, source scans, CI summary keys, rendered JSON, and projection rows are not semantic authority.

Process fungibility is part of this boundary. Certified runs are store-owned durable work, not
process-owned work. Run identity comes from `RunAdmitted`; execution lanes only choose the current
live driver for a base work identity; resource lanes protect certified side effects. Process-local
stores, transports, signer providers, and driver loops cannot define run identity, side-effect
authority, replay authority, public-output authority, or terminal status.

Public run-start ingress uses an exact entry-point id plus one stable target, as described in
`docs/design.md`. Current-configuration resolution is an app pre-admission concern; it is not
runtime or replay authority.

The portfolio model has one direct `HoldingSourceConfig` algebra: `Native` or EVM `Erc20` with a
normalized non-zero contract address. EVM native scale belongs only to `NetworkConfig::Evm`.
Bitcoin collection performs one bounded multi-descriptor scan per semantic source, verifies one
shared height/hash anchor, and emits ordered balance facts plus a checked receipt from one read
state. Each EVM network becomes one child call to the reusable `EvmBalanceCollectionOperation`,
whose single fact-producing read state returns a checked receipt with its ordered
`evm.balance_snapshot` facts in one settlement. The typed Bitcoin/EVM receipt vectors flow directly
into one store-backed selection state; assembly
receives only rehydrated and identity-reverified facts. The complete snapshot graph is the sole
public objective, `mfm.portfolio/snapshot@2`.

`mfm-op-portfolio-snapshot` owns that complete internal graph through two operations.
`PortfolioSnapshotOperation` projects normalized `PortfolioConfig` into child Bitcoin/EVM
collector calls and one `PortfolioReportOperation` call; it constructs no state directly. The
report operation receives the typed family receipt handles and owns receipt-pinned selection,
snapshot assembly, and report projection. Its one production draft helper binds exactly one
`PortfolioPublicOutputs` root. Portfolio admission bounds networks, wallets, symbols,
wallet-to-symbol relations, and distinct EVM sources per network before graph expansion. The app
registers the needed runners and certification descriptors, strictly resolves one target-keyed
`PortfolioConfig` at admission, and exposes that exact graph only through
`mfm.portfolio/snapshot@2`.

The snapshot operation registry composes the Bitcoin and EVM child registry functions, then adds
only the report operation's own states and operations. Parent registries must never repeat a child
crate's concrete inventory: child topology changes flow through the registry-composition primitive
into authoring and certification together.

## Semantic Package Metadata

Cargo metadata records package semantics, not directory taxonomy or a frozen package inventory.
Every workspace package declares exactly one `package.metadata.mfm.layer`:

| Layer | Package responsibility |
|---|---|
| `kernel` | Domain-free platform contracts and runtime infrastructure. |
| `domain` | Pure domain model, capability, signing, state, or operation responsibilities. |
| `live` | Domain adapter and transport implementations. |
| `signing` | Generic signing contracts below domain code. |
| `secret-provider` | Secret-bearing keystore and signer implementations. |
| `storage` | Concrete storage implementations. |
| `assembly` | Process configuration, implementation construction, and application services. |
| `binary` | CLI or API executable surfaces. |
| `test` | Unrestricted test-only support. |

Domain packages also declare a validated lower-kebab `domain` and a `domain-role` of `source` or
`aggregate`. Every package for one domain must agree on its role. Live packages declare the domain
and derive its role from the matching pure-domain packages; a live package without a pure-domain
owner is invalid.

Kernel packages declare whether domain code may depend on them with `domain-facing`. Kernel and
assembly packages declare whether binaries may depend on them with `binary-facing`. These flags are
typed booleans and are required even when false. Other layers cannot carry them. Metadata keys are
closed: path/category aliases, phase fields, package allowlists, and named exceptions are invalid.

Cargo target kinds constrain metadata. A non-test package with any binary target is layer `binary`,
and a mixed library/binary package applies the binary dependency row to the whole package. Proc
macros remain in dedicated non-binary packages. Package renames, moves, additions, and deletions do
not change this contract and do not require an inventory test update.

## Authority Contract

The typed boundary separates data, evidence, authority, and implementation artifacts:

- parsed typed spec JSON enters as `UntrustedTypedSpec` and is data only
- `LoweredTypedSpec` is program-lowered draft data, not runtime authority
- `ValidatedTypedExecutionSpec` is certifier-private validated authority used to mint certificates
- `HashedSpecEnvelope` is a hash-only envelope only
- persisted `CertifiedSpecCertificate` bytes are evidence only until verified
- `CertifiedTypedSpec` is the non-forgeable authority returned by `mfm-certify`
- `CertifiedDescriptorSet` and `CertifiedFrameworkLifecycle` are certified spec authority views
- `CertifiedContextSpec` and `ContextRef` are hash-defining spec data until certifier/runtime
  authority validates them with node, cell, and input context constraints
- `CertifiedRuntimeSpec` is runtime authority derived only from `CertifiedTypedSpec`
- `PreparedCommit<Purpose>` and `PreparedCommitPlan` are store mutation authority built by runtime
- `CertifiedRunStoreAuthority` is minted from the certified typed spec and is the store admission authority for
  policy-bound run-start and saga commits, including side-effect terminal policy derivation; store
  admission must match it to the projected `RunAdmitted.spec_hash`
- `ManualResolutionProofAuthority` and `VerifiedManualResolutionForPrefix` are manual proof
  authority over a certified blocked prefix
- `CertifiedSideEffectContract` is shared live, resume, and replay authority for side-effect claims
- `SideEffectLedgerState` is the typed store view for legal side-effect ledger transitions
- `SagaTerminalProof` is required authority for terminal saga outcomes
- `CommittedRunStream` is store-owned append-only stream authority
- `VerifiedRunArtifactStore` is retained-artifact authority tied to a committed stream
- `VerifiedRunHistoryView` is runtime/replay read authority over a committed stream plus verified
  retained artifact evidence
- erased runner plans are implementation artifacts
- rendered public-output JSON is an output/cache surface only

Start, resume, replay, and public-output rendering must verify stored spec/certificate artifacts
against the compiled certification registry and compare stream evidence before constructing runtime,
replay, or render authority.

Admission, drive, verify, resume, and replay must not resolve outcome-affecting policy from mutable
registries, worker-local defaults, or external oracles. Such policy belongs in typed config and
certified spec material before runtime authority is constructed.

Use `docs/persisted-public-surfaces.md` when reviewing data that is persisted, returned by CLI/REST,
or exposed through app read paths. It classifies allowed data, forbidden secret classes, provenance
authority, and tests for each surface.

## Responsibility Taxonomy

Every new unit must declare which responsibility it owns before it gets a module, crate, schema
namespace, capability, public type, CLI command, REST route, or test fixture. Responsibilities do
not automatically become packages; one pure domain crate may preserve several roles through private
modules, one-way source dependencies, narrow exports, and focused tests.

| Category | Owns | Does Not Own |
|---|---|---|
| Platform primitive | Reusable infrastructure such as signing, protocol clients, source routing, artifact access, process execution | Workflow topology or domain-specific semantics |
| Kernel | Framework-owned typed authority contracts such as specs, events, store, runtime, replay, and manual authorization proof contracts | Domain semantics, live IO, signer providers, storage implementations |
| Capability contract | Typed authority contracts such as capability specs, request/response evidence types, redacted errors, and traits consumed by states/adapters | Live IO, endpoint routing, signer material resolution, workflow topology |
| Domain model/config | Pure domain types, validation, canonical config, schema descriptors | Runtime IO, signer resolution, transport clients |
| State | Reusable executable domain semantics and typed state contracts | Ambient IO, app/store authority, protocol implementation |
| Operation | Deterministic graph topology and config lowering | Runtime execution, IO, signers, transports, replay |
| Adapter | Runner binding from state intent to capabilities and evidence recording | Generic protocol clients, signer provider internals, operation topology |
| Transport | Reusable protocol implementation and live/replay capability backend | Workflow recipes, signer material, domain topology |
| Signer | Generic signer refs, signing requests/results, signing provider implementations | Workflow recipes, raw signed transaction persistence |
| Storage | Typed run-event or artifact persistence implementation | Domain semantics, scheduler behavior |
| App assembly | Registry/store/artifact/capability wiring and typed run services | Workflow planning or state behavior |
| Binary/API | Input decoding, routing, response envelopes | Domain semantics, runtime authority, direct state execution |

Capability contracts are foundational protocol authority. They remain below states whether the
contract and state are separated by Cargo or by private modules inside one pure-domain package.
Domain models may consume lower checked protocol identities, but capability code never depends on
higher state or operation modules. If a unit does not fit one responsibility cleanly, the design is
not ready.

### Current Configuration And Runtime-Config Boundary

The semantic configuration path has one ownership split:

- `bin/cli` and `bin/rest-api` decode an entry-point id and target only;
- `mfm-app` owns the closed setup TOML document, typed validation, canonical JSON, prohibited-field
  scanning, target-keyed current-configuration persistence, and target resolution;
- operation crates receive concrete typed values and deterministic joins; completed configs and
  graphs do not contain target indirections;
- configuration storage persists opaque canonical rows keyed by stable target and knows neither
  domain config nor setup kinds;
- kernel, state, adapter, transport, runtime, replay, and binaries do not depend on configuration
  model types or configuration persistence.

Target resolution ends before certification and `RunAdmitted`. Launch evidence records the exact
entry-point id plus target/schema/digest so a run can be verified without consulting mutable
current configuration. Resume, status, stream, public-output, and replay paths use retained
certified artifacts and the append-only run stream only.

Two current decisions are deliberate:

- Family collector receipts are the completion authority for portfolio selection.
  `PortfolioSnapshotOperation` passes typed Bitcoin and EVM receipt vectors into one
  `PortfolioReportOperation`, whose structured input is passed unchanged to
  `SelectHoldingsState`; neither operation has a generic receipt entry, logical-manifest wrapper,
  count/readiness value, or fan-in state. Selection checks
  exact portfolio demand and both family receipt contracts before issuing one shared-snapshot query
  batch. The portfolio state package's dependencies on the Bitcoin and EVM state packages are the
  explicit downstream typed-output/fact contracts; it owns neither family's runner, transport,
  fact publication, replay, or workflow topology.
- Runtime TOML remains a process-local routing and signer boundary rather than semantic
  configuration data. Live assembly selectively resolves only the requested EVM route or requested
  signer plus its referenced keystore, so malformed unrelated entries do not block that resource.
  Read-only paths do not load it, and it is never persisted or used by replay. There is no
  compatibility path or hidden fallback.

## Boundary Contract

### Operation

Operations are deterministic planning only.

Operations may:

- parse and validate typed planning config
- lower validated config into canonical typed config
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
- depend on capability contract crates that define typed authority contracts
- define hash-bound external-read plans/evidence or side-effect contracts when applicable
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
runtime/app assembly. External-read states own a deterministic plan and reducer; adapters execute
the plan but do not own replay semantics. State code must not create its own live network,
filesystem, clock, process, or signer access when that access is part of semantic execution.

### State Capability Boundary

States declare authority. Transports implement authority. Adapters bind the two at runtime.

States may depend on capability contract crates because those crates define typed authority
contracts. States must not depend on live transport implementation crates. Adapters translate
state-owned plans or mutation intent into capability calls and recorded evidence. The generic
runtime runner owns materialization, reduction, and evidence staging for ordinary pure states and
external reads; the generic replay driver loads the same typed evidence and calls the same pure
behavior or read reducer. Domain adapters do not copy pure runner or replay implementations.
Transports perform protocol IO and implement capability contracts.

For replay/resume semantics, including pure/read/side-effect behavior, see the authoritative
`State Capability Boundary` and `Certified Saga Semantics` sections in `docs/design.md`.

### Adapter

Adapters connect state-owned domain intent to runtime capabilities.

Adapters may:

- bind certified state descriptors to executable runners
- materialize typed inputs through runtime-provided surfaces
- execute state-authored external-read plans without redefining their reducers
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

Adapter code belongs under `crates/adapters/*`. A workflow-specific runner crate is not a license
to own generic transport or signer behavior.

### Transport

Transports implement reusable protocols and live/replay capability backends.

Live transports that satisfy capability contracts expose bound providers, not raw routers or
unchecked clients. App/adapters derive certified semantic bindings, bind providers once per source
intent, and issue operation-only requests. Provider implementations own the mandatory validation
path: route/source resolution, live identity probes, operation checks, response identity checks, and
redacted diagnostics must be non-bypassable. Raw capability IO must remain unreachable without a
private provider-minted verified call/session value; do not add public middleware, decorators,
`inner`, `unchecked`, or `skip_validation` APIs for production transport authority.

Transports may:

- consume typed runtime source and route descriptors supplied by app assembly
- select protocol sources by non-secret runtime refs
- redact endpoints and authorization material
- execute protocol calls
- implement protocol traits defined by capability contract crates
- support multiple adapters, states, and workflows
- answer replay requests from recorded evidence only when implementing replay backends

Transports must not:

- use workflow recipe names
- define the state-facing protocol contract that states depend on
- depend on workflow operation crates
- know about workflow topology
- open keystores or own signer behavior
- read password files
- persist typed semantic events by themselves
- leak RPC URLs, authorization headers, or secret-bearing local routing details into typed semantic
  surfaces

If two workflows can use the same protocol behavior, that behavior belongs in a shared transport
before either workflow lands.

### Signer

Signers provide generic key material and signature capabilities.

Signer crates may:

- define signer references
- define protocol-neutral signing algorithm, profile, domain, and purpose identifier types
- define signing request/result traits
- implement MFM keystore-backed signing
- map runtime signer refs to keystore entries, hardware signers, remote signers, or future wallets
- verify expected public identities
- enforce an explicitly requested deterministic/canonical provider profile
- redact all secret-bearing details from errors

Signer crates must not:

- depend on workflow operation crates
- depend on domain lifecycle state crates
- know about CLI or REST request shapes
- persist private keys, mnemonics, passwords, password paths, keystore paths, signature scalars, or
  raw signed transaction bytes in typed semantic surfaces

Raw signed transactions are bearer mutation material. They remain transient submit-time bytes below
the typed semantic boundary. `mfm-evm-signing` owns the sole domain-specific EIP-1559 envelope
conversion/finalization path: Alloy supplies the signing digest, signed encoding, and transaction
hash; MFM verifies the generic result profile, canonical low-s/parity, and recovered sender. The
keystore provider is protocol-neutral and must not import EVM domain or purpose constants.

The app layer may resolve one exact runtime signer binding and referenced keystore profile, build
the generic provider, and call the canonical EVM signing function. Binaries may parse typed input,
invoke that app service, and publish the returned bearer only to an explicit user-selected file;
they must not open a signing key, construct a second signing path, or retain the bearer.

### Configuration

Typed config contains semantic intent and non-secret references only.

Typed config may contain:

- domain intent
- content-addressed artifact refs or inline non-secret artifacts
- network id
- source/oracle id only when source selection is domain intent, not local runtime routing
- expected chain id or equivalent protocol identity
- signer ref
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

### Multi-Network Workflow Symmetry

Similar network-backed workflows should share workflow roles where the domain semantics actually
match, but symmetry is not an architecture category and must not introduce generic cross-network
types by itself.

Enforce the hard boundaries instead:

- operations keep topology network-neutral when the workflow semantics are network-neutral
- domain models and states name family-specific semantic fields explicitly
- states emit deterministic read or mutation intent, not live transport requests
- adapters bind providers from state intent and issue family-specific operation-only capability
  requests
- transports own reusable protocol behavior, route resolution, redaction, and live provider checks
- tests cover each supported family at the model, state-intent, adapter, transport, and registry
  binding surfaces that family actually uses

Do not add a generic transport, guard, or adapter abstraction solely because two families occupy the
same workflow slot. Add shared code only when it removes real duplication without erasing protocol
semantics. When a family cannot provide the same semantic guarantee, fail closed or design a new
explicit workflow mode rather than weakening the existing mode.

## Architecture Doctrine

Apply the repository-wide [one-current-design policy](code-quality.md#one-current-design).
Package count alone is not a simplification metric when reducing it would merge distinct authority
boundaries.

### Rule 1: One Coherent Ownership Boundary Per Crate

A crate must own one coherent reusable or enforcement boundary. Architectural roles remain
distinct, but a role alone is not sufficient reason for a package. Use private modules and
visibility when Cargo isolation would add only manifests, bridge types, and public plumbing.

Good crate reasons:

- a reusable protocol transport colocated with its domain adapter behind a public/private module boundary
- reusable signer provider
- a pure domain whose model, capability, state, and operation roles share one bounded dependency surface
- setup/canonical config pipeline
- storage implementation
- runtime/kernel primitive
- a dependency, proc-macro, process-assembly, or secret-bearing firebreak that Cargo must enforce

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

Current authority names include:

- `mfm.evm.read`
- `mfm.evm.transaction`
- `mfm.signing.sign`

Disallowed:

- feature-specific names for generic EVM RPC reads
- workflow-specific names for generic signer access
- recipe-specific names for transaction submission

The effect/capability role count can be correct while the capability taxonomy is wrong. Reviewers
must check both.

### Rule 4: Public APIs Use Domain Language

Public names should describe what the user means, not how the implementation was assembled.

Allowed domain terms in public documentation:

- EVM transaction creation action
- EVM transaction call action
- exact-anchor contract validation state
- EVM transaction intent
- EVM RPC source
- signer reference
- signing provider

Disallowed public names:

- temporary acronyms
- parity fixture names
- implementation recipe names
- transport crate names exposed as route or command names

Direct creation and ordinary calls share one transaction state; operation crates own
any domain-specific deployment or configuration topology. A reusable descriptor still does not
imply a public operation or setup surface, and the app publishes only certified objectives with a
current consumer.

`SubmitEvmTransactionState` remains reusable library/test substrate. Its runner preclaims the exact
`mfm.evm.sender_nonce` lane, prepares one immutable EIP-1559 envelope, and delegates
submission/observation to one adapter contract. Preparation admits intent, exact nonce, and checked
fees into one complete type-2 estimate request before IO, then adds the returned gas limit to that
same representation for Alloy signing.

`ValidateEvmContractState` remains an independent signer-free library/test foundation. Its plan
fixes one address/number/hash anchor, mandatory non-empty runtime-code hash, and bounded ordered
full-context calls. Its adapter binds one checked read session, executes code and calls at the exact
hash, and finishes with a number-to-hash canonicality read. Live execution and evidence-only replay
both use the state reducer; replay never binds a route or session.

The adapter exposes separate balance, validation, and transaction registration functions. The
production app invokes balance registration only and omits validation and transaction descriptors,
runners, and replay dispatch. An explicit library consumer or test must assemble either omitted
foundation itself. Product assembly is validated against its published authoring contract rather
than inferred from package names.

### Rule 5: Runtime Routing Is Not Semantic Config

Semantic config describes domain intent. Runtime process config chooses concrete local resources.
Typed workflow config must not contain process-local source routing unless choosing the source is
itself domain intent, such as selecting a named oracle.

Keep these concepts distinct:

- `network_id`: semantic domain network label
- `expected_chain_id`: concrete chain identity returned by the network
- `source_identity`: non-secret source selection, only when the selected source is domain intent
- `signer_ref`: non-secret signer reference

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

- New domain-free semantic primitive: a `kernel` package, with no domain dependency.
- New pure model, capability, signing, state, or operation behavior: the owning `domain` package and
  its corresponding private role module.
- New domain protocol IO or runtime binding: the owning `live` package; transports remain public
  reusable modules and adapters remain private registration modules.
- New generic signer contract: `signing`; secret-bearing implementation: `secret-provider`.
- New concrete store: `storage`; the store contract remains `kernel`.
- New process construction or application service: `assembly`.
- New command/API shape: `binary`, backed by binary-facing assembly services.
- Cross-package fixtures and parity harnesses: `test`.

Directory names are navigation aids. Metadata and dependency behavior, not a suffix or path, prove
placement.

## Dependency Rules

Kernel crates point inward only through the kernel dependency DAG:

```text
ids -> canonical -> values -> capabilities
  -> program/spec -> certify/events/store/manual-auth -> runtime/replay
```

Normal and build dependencies use this semantic direction:

- `kernel` -> `kernel`;
- `signing` -> domain-facing `kernel`;
- source `domain` -> domain-facing `kernel`, `signing`, and same-domain packages;
- aggregate `domain` -> domain-facing `kernel`, `signing`, same-domain packages, and source-domain
  packages;
- source `live` -> `kernel`, `signing`, its source-domain packages, and same-domain live packages;
- aggregate `live` -> `kernel`, `signing`, its aggregate domain, source domains, and same-domain
  live packages, never source-live packages;
- `secret-provider` -> domain-facing `kernel`, `signing`, and `secret-provider`;
- `storage` -> `kernel`;
- `assembly` -> lower layers and assembly support;
- `binary` -> binary-facing `kernel` or binary-facing `assembly`; and
- `test` -> unrestricted.

Source-domain to aggregate-domain, cross-domain live, live to concrete storage/secret/app, signing
to secret-provider, and store-contract to storage-implementation edges are forbidden. Dev-only
dependencies may exercise lower surfaces without becoming production ownership.

The ordered repository cut currently has two broader semantic shapes which close with their owning
deletions: Bitcoin source state still reaches the aggregate portfolio contract until the atomic
collection replacement, and binaries may still reach non-facing kernel/assembly plus the secret
provider until app services absorb implementation construction. These are not named exceptions or
approved future edges. The metadata evaluator narrows monotonically at those cutovers.

## Store Boundary

`mfm-store` defines the production commit contract. Implementations accept only
`PreparedCommitPlan` values built from purpose-specific `PreparedCommit<Purpose>` authority for
execution mutation. Each prepared commit carries typed payloads and artifact evidence to admit
atomically with those payloads. There is no public raw prepared-commit constructor or append method;
new commit purposes must add a purpose marker and validator before stores will accept them.

Stores own:

- event envelopes
- event-to-retained-artifact requirement derivation
- run-local sequence numbers
- the durable store-wide append coordinate used by cross-run projections
- ordinals
- event ids
- logical keys
- commit preconditions
- retained artifact bytes and typed evidence
- the retained-artifact read provider and verified-byte contract
- projections

Projection data is derived from the run stream and is never the sole authority for semantic resume,
replay, retention, public output, or side-effect status.

Synthetic store mutation is reserved for explicitly named non-execution test, migration, repair,
corruption, or low-level storage contract fixtures.

## App And Binary Boundary

`crates/app` wires:

- typed runner registries
- typed capability backends
- the production Postgres run store
- the store-owned retained-artifact reader
- certified start/resume/replay services
- typed public-output rendering

`crates/app` must not own workflow planning, state behavior, or adapter runner behavior. It may
construct concrete process-local resources such as the Postgres run store, protocol clients, and
signer providers, then pass them into adapter-owned runner factories.
Evidence-only app services for status, stream inspection, list/watch, replay, and public-output
rendering must be constructible from store, artifact, and certification/replay authority only. They
must not construct live EVM transports, signer providers, or live capability runtime config. Live
start/resume services may construct those live drivers because they are execution authority.

Live app assembly also computes one current-executable byte identity before constructing the runner
registry. The registry mints all logical factory bindings—including framework, pure, external-read,
and adapter factories—from that one template, so factory ids remain distinct while their executable
digest is identical. Runtime and domain packages accept those bindings and do not derive process
identity. Evidence-only services do not construct the registry and therefore perform no executable
file access.

`bin/cli` and `bin/rest-api` may:

- decode JSON/TOML/user input
- build typed configs and certified specs through operation crates
- construct app services over the Postgres run store
- start, resume, replay, inspect, and render typed runs
- read run list/watch observations through the shared app API
- emit only the current response envelopes documented by the relevant binary

Read-only CLI/REST commands and routes must use evidence-only app services. Live start, resume, and
manual-resolution drive paths may use live app services that bind runners and capabilities.

`bin/cli` and `bin/rest-api` must not:

- plan workflow semantics directly
- execute state behavior directly
- bypass typed certification
- infer public outputs from untyped snapshots
- accept uncertified run history as typed run authority
- depend directly on SQLx, filesystem artifact stores, or alternate production storage selectors

## Public Naming Rules

Public CLI commands, REST routes, request kinds, schema ids, error codes, executable identities, and
public-output schema ids are exact identities in the current contract. They carry no cross-revision
compatibility promise.

Before adding or renaming a public surface, verify:

- the name is domain language, not implementation scaffolding
- the name would still make sense if the current operation topology changed
- the name does not expose an adapter, transport, crate, parity fixture, or temporary acronym
- the schema namespace is domain-specific enough to persist in current artifacts and run streams
- tests assert the public domain name and reject implementation-scaffolding names

## Review Checklist

Before merging a change, verify:

- the new unit has one clear responsibility and its package has one semantic layer
- the crate boundary enforces a real reusable, build, process, proc-macro, storage, or security boundary
- typed specs remain the only runtime contract
- new public values/configs use typed descriptors and no floats/secrets
- side effects have one state-authored intent/idempotency pair, required typed prepared authority,
  typed submission and receipt/recovery evidence, adapter-only mutation IO, and no retained signed
  raw transactions
- deterministic signing requirements are explicit profile ids checked by callers and providers,
  with no unconstrained or fallback signer binding
- certified saga and side-effect verification policy are hash-defining spec data, not policy
  resolved by a registry at admission, and compensation/manual outcomes are derived from certified
  policy plus stream evidence
- process topology remains operational: run identity, execution lanes, and resource lanes keep
  separate responsibilities, and process-local resources do not become durable authority
- manual resolution uses certified schema roles, certified verifier identity, certified operator
  authority snapshot, canonical proof bytes, signature verification, and quorum
- replay paths cannot construct live capabilities
- replay paths cannot call live signers, verifier registries, certification registries, keystores,
  or runtime signer sources
- admission, drive, verify, and resume paths cannot call mutable policy registries or external
  oracles for outcome-affecting decisions after certification
- resume validates stored stream evidence against the certified spec
- states declare capabilities but do not instantiate live transports or signer providers
- transports implement capability contracts but do not define state-owned domain semantics
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

Required metadata checks assert:

- closed, typed layer/domain/role/facing metadata without name, path, count, or exception tables;
- agreement of every domain role and a pure owner for every live package;
- the semantic dependency matrix for normal and build dependencies;
- binary target/layer coherence, including mixed library/binary packages;
- a dedicated non-binary proc-macro boundary; and
- positive and negative synthetic fixtures for every dependency row.

## Companion Docs

- `docs/design.md`: authoritative runtime, store, replay, and secret-handling contract
- `docs/saga.md`: certified saga, scoped AC/DC language, manual authorization, and resource-claim
  contract
- `docs/code-quality.md`: mandatory quality policy for code, test, documentation, build, and
  workflow changes
- `docs/build-and-verification.md`: scope-driven development workflow, build lanes, and gate
  contract
- `bin/cli/README.md`: CLI command and JSON output contract
- `bin/rest-api/README.md`: REST contract
