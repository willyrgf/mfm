# RFC: Repository Reorganization

- Status: Proposed
- Date: 2026-07-21
- Scope: Workspace package boundaries, production registration, and shared runner infrastructure

## Summary

MFM currently has 44 workspace packages. Many packages represent architectural roles such as
capability, state, operation, adapter, or transport rather than independently reusable abstractions
or necessary dependency firebreaks. The result is a wide dependency graph, duplicated execution and
replay plumbing, an app assembly crate coupled to most of the workspace, and architecture tests that
preserve package inventory instead of architectural properties.

This RFC proposes reducing the workspace to approximately 27 packages. Pure Bitcoin, EVM, and
portfolio behavior will each live in one domain crate. Process-local implementations will live in
one corresponding live-integration crate per domain. Small kernel packages will be consolidated
where their responsibilities are already inseparable. Unused production workflows and topology
will be deleted.

Capability, state, operation, adapter, and transport remain useful architectural roles. They stop
being automatic reasons to create packages.

The projected package count is an outcome, not an invariant. Tests must enforce dependency and
registration properties instead of an exact number or exact package-name inventory.

## Decision principles

This reorganization follows these priorities, in order:

1. Fewer concepts.
2. Fewer execution and replay paths.
3. Fewer public types and schemas.
4. Fewer duplicated responsibilities.
5. Fewer places a future change must touch.
6. Fewer lines of code after correctness, security, and replay guarantees are preserved.

The implementation does not preserve backward compatibility. When a package, type, schema, or code
path is replaced, the old form is deleted in the same commit. The migration must not create
compatibility crates, deprecated aliases, re-export facades, feature-gated legacy paths, fallback
readers, or parallel old and new implementations.

A package is justified when Cargo must enforce a dependency prohibition that a module cannot, or
when the package is an independently reusable platform primitive with a coherent public contract.
Repository taxonomy alone is not sufficient justification.

## Problem situation

### Architectural taxonomy became package topology

The repository distinguishes domain models, capabilities, states, operations, adapters,
transports, signers, storage, and app assembly. Those distinctions are useful for assigning
responsibility, but the current workspace commonly assigns each role its own package.

The EVM balance path illustrates the result: capability contracts, signing, states, operations,
adapters, and transports are six separate packages even though they form one bounded domain used by
one product assembly. Bitcoin and portfolio follow similar patterns.

This causes ordinary domain changes to cross several manifests and public APIs. It also encourages
types to become public merely because adjacent packages must exchange them.

### Capability packages do not represent one consistent abstraction

The top-level `*-capabilities` packages have different meanings:

- `mfm-artifact-capabilities` wraps store-owned retained-artifact authority and duplicates the
  store's artifact-evidence shape.
- `mfm-fact-capabilities` defines ports and markers that operate directly on `mfm-facts` types.
- `mfm-btc-capabilities` and `mfm-evm-capabilities` contain protocol-domain identities, evidence,
  requests, errors, and provider contracts.
- `mfm-capabilities` is the kernel authority model and re-exports the separate `mfm-effects`
  package.

The common suffix therefore does not indicate a common ownership boundary. It scatters related
domain types while placing already-coupled kernel concepts in separate packages.

### Shared runner infrastructure is incomplete

The runtime has a generic external-read runner, but no equivalent generic runner for pure states or
managed fact writes. Domain adapters consequently repeat configuration and input loading, output
construction, fact staging, commit preparation, and replay verification.

Adding another adapter package does not solve this duplication. The missing support belongs in the
kernel runner kit. Once that support exists, adapter-specific copies must be deleted.

### Production assembly contains disconnected topology

The app publishes exactly one run entry point: `mfm.portfolio/snapshot@1`. Production registries also
contain descriptors and runners that are not reachable from that graph, including:

- the proof workflow;
- the standalone Bitcoin chain-head/checkpoint collector cycle;
- the standalone EVM balance-cycle launch wrapper; and
- reusable EVM transaction and validation substrate that has no published operation.

Reusable library primitives do not need automatic production registration. Registration should
follow the published graph. Tests for reusable primitives can construct explicit local registries.

The EVM transaction and exact-anchor validation primitives remain deliberate platform foundations;
they will be retained inside the EVM domain and live-integration crates. They will not be registered
by production assembly until a published operation consumes them.

The proof workflow, Bitcoin checkpoint cycle, and EVM standalone cycle wrapper have no current
product owner and will be deleted. Test coverage for kernel invariants must use minimal test-local
fixtures rather than production packages.

### Architecture tests preserve accidents

The Cargo metadata contract currently asserts an exact 44-package workspace, an exact six-package
EVM inventory, path-based category exceptions, and named dependency overrides. These assertions
make simplification look like an architecture violation even when dependency boundaries improve.

Architecture tests should reject forbidden dependencies and unreachable production registrations.
They should not preserve directory names or manifest counts.

## Goals

- Eliminate top-level domain `*-capabilities` packages.
- Keep pure domain behavior reusable without the CLI, app, live IO, or storage implementations.
- Keep live Bitcoin, EVM, and portfolio wiring isolated from one another.
- Centralize ordinary pure-state, external-read, and managed-fact-write execution paths.
- Remove dormant production workflows and registrations.
- Reduce public types and schemas that exist only to cross package boundaries.
- Preserve thin CLI and REST binaries.
- Preserve all security, canonicalization, event-store, replay, and atomicity invariants.

## Non-goals

- Creating one monolithic MFM library.
- Merging pure domain logic with live IO.
- Merging generic signing contracts with secret-bearing keystore implementation.
- Merging store contracts with the Postgres implementation.
- Merging the proc-macro package into a runtime library.
- Adding public entry points, workflow modes, protocol fallbacks, or speculative extension points.
- Reading or migrating runs produced by the old package/type/schema topology.

## Proposed package structure

```text
crates/
|-- kernel/                         # 14 packages
|   |-- ids
|   |-- canonical
|   |-- values
|   |-- capabilities               # absorbs effects
|   |-- facts                      # absorbs fact-capabilities
|   |-- program
|   |-- program-derive
|   |-- spec
|   |-- certify
|   |-- events
|   |-- store
|   |-- manual-auth
|   |-- runtime
|   `-- replay
|-- domains/
|   |-- bitcoin                    # model, capability, state, operation
|   |-- evm                        # model, signing, capability, state, operation
|   `-- portfolio                  # model, state, operation
|-- live/
|   |-- bitcoin                    # runtime config, transport, runner binding
|   |-- evm                        # runtime config, transport, runner binding
|   `-- portfolio                  # fact/artifact providers and runner binding
|-- signing                        # generic signer contract
|-- keystore                       # secret storage and signing provider
|-- storages/
|   `-- postgres
`-- app

bin/
|-- cli
`-- rest-api

tests/
`-- integration
```

This structure projects to 27 workspace packages:

| Group | Packages |
|---|---:|
| Kernel | 14 |
| Pure domains | 3 |
| Domain live integrations | 3 |
| Signing, keystore, and Postgres | 3 |
| App | 1 |
| Binaries | 2 |
| Integration tests | 1 |
| Total | 27 |

The number 27 is not tested. A later consolidation or separation is valid if it follows the package
boundary rule and improves the architecture properties in this RFC.

### Why live integration remains per domain

A single process-wide live crate would have fewer manifests, but it would permit Bitcoin, EVM, and
portfolio wiring to acquire dependencies on each other. Three small live-integration crates create a
useful Cargo-enforced prohibition while allowing the app to assemble them.

Each live-integration crate may own its process-local configuration, concrete transport, provider
implementation, and runner registration. It must not own deterministic operation topology or
domain reduction semantics.

### Roles inside domain crates

Pure domain crates should use private or narrowly public modules for their internal roles:

```text
mfm-bitcoin
|-- model
|-- capability
|-- state
`-- operation
```

The state/operation rule still applies: state modules own reusable execution semantics, while
operation modules own deterministic graph construction. A module boundary is sufficient because
both are pure, domain-owned code and share the same allowed dependencies.

## Package disposition

| Current packages | Disposition |
|---|---|
| `mfm-effects` | Absorb into `mfm-capabilities`; delete the old package. |
| `mfm-fact-capabilities` | Absorb into `mfm-facts`; delete the old package. |
| `mfm-artifact-capabilities` | Move the useful fact-artifact helpers to their store/fact owners; delete the package and app adapter facade. |
| `mfm-btc-capabilities`, `mfm-states-btc`, `mfm-op-btc-collectors` | Consolidate as `mfm-bitcoin`. |
| `mfm-adapters-btc-jsonrpc`, `mfm-transports-btc-jsonrpc-http` | Consolidate as `mfm-bitcoin-live`. |
| `mfm-evm-capabilities`, `mfm-evm-signing`, `mfm-states-evm`, `mfm-op-evm-collectors` | Consolidate as `mfm-evm`. |
| `mfm-adapters-evm`, `mfm-transports-evm` | Consolidate as `mfm-evm-live`. |
| `mfm-portfolio-model`, `mfm-state-portfolio`, `mfm-op-portfolio-snapshot` | Consolidate as `mfm-portfolio`. |
| `mfm-adapters-portfolio` | Becomes `mfm-portfolio-live`. |
| `mfm-runtime-config` | Split process-local sections between domain live crates and app assembly; delete the package. |
| `mfm_core`, `mfm-signers-keystore` | Consolidate and rename as `mfm-keystore`. |
| `mfm-collectors-proof`, `mfm-op-proof`, `mfm-transports-proof` | Delete completely. |

All packages not listed remain separate unless a later RFC or independently reviewable change
demonstrates that their boundary is unnecessary.

## Execution-path consolidation

### Generic pure-state runner

`mfm-runtime` will own one generic `PureStateRunner`. It will perform the standard typed state
sequence:

1. Load and validate certified state configuration, inputs, and context.
2. Invoke deterministic state behavior.
3. Build canonical outputs and artifacts.
4. Return one runtime-owned settlement shape.

Every adapter-owned implementation of that same sequence must migrate in the introducing commit and
be deleted.

### Generic managed-fact-write runner

`mfm-runtime` will own one generic `ManagedFactWriteRunner`. A managed fact-write state will produce
its typed output and complete fact batch. The runner will validate and stage both, and the runtime
will commit them atomically through its existing append authority.

This removes Bitcoin and EVM copies of fact-record runner mechanics. Managed store authority remains
inside the runtime/store boundary; domain state code does not gain ambient IO.

If certification needs a stable managed-fact-write identity, that identity remains runtime-owned. A
public provider abstraction or marker type must not survive solely to preserve the former package
shape.

### Replay

Pure states and ordinary managed fact writes will use generic evidence-only replay paths paired with
their generic runners. Protocol-specific replay remains only where protocol semantics require it,
such as EVM side-effect evidence.

Replay must never construct live transports, load current runtime configuration, resolve signers, or
perform network IO.

## Bitcoin collection simplification

The active portfolio Bitcoin collection will become one operation with two states:

```text
BitcoinCollectionOperation
  -> CollectBitcoinBalancesState       # external read
  -> RecordBitcoinBalanceFactsState    # atomic managed write and receipt
```

The external-read state will:

- bind one checked source;
- resolve one canonical tip;
- read every strictly sorted configured address against that tip; and
- return one canonical evidence batch.

The managed-write state will validate the complete batch, record all address facts atomically, and
return the collection receipt.

This deliberately trades per-address retry granularity for one anchored observation, one retained
evidence shape, one fact commit, and a much smaller certified graph. A failed read retries the batch;
it never exposes a partially collected snapshot.

Fields whose values are fixed or derivable, including constant coverage/status values and authored
read-count limits, will be removed from public configs and outputs. Read budgets will be derived from
the validated request shape.

The following standalone surface will be deleted:

- checkpoint query and record states;
- checkpoint and standalone chain-head fact types;
- `BtcChainHeadCollectorCycleOperation`;
- its draft and launch helpers; and
- dedicated receipt assembly used only by the old graph.

Chain-head resolution remains an internal semantic step in the active aggregate read.

## Production registration

Production certification and runner registries must contain only descriptors reachable from a
published entry-point graph plus runtime-owned framework primitives required to execute any graph.

Library tests may register reusable states directly. Library availability is not a reason for the
app to register a state globally.

Under this rule:

- portfolio Bitcoin and EVM collection remain registered through the portfolio snapshot graph;
- proof and standalone collector cycles disappear;
- EVM transaction and exact-anchor validation remain tested reusable library primitives but are not
  registered by production app assembly until a published operation consumes them.

## Dependency contract

Architecture tests will enforce these properties:

1. Kernel crates depend only on permitted kernel crates and external libraries.
2. Pure domain crates never depend on app, binaries, live integrations, keystore, or Postgres.
3. Domain live-integration crates may depend on their corresponding pure domain and kernel/platform
   contracts, but never on another domain's live integration.
4. Generic signing contracts never depend on the keystore implementation.
5. Store contracts never depend on Postgres.
6. Secret-bearing keystore implementation never leaks into persisted domain schemas.
7. Binaries remain thin and delegate run assembly and execution to `mfm-app`.
8. The proc-macro package remains separate from runtime library packages.
9. Every production-registered domain descriptor is reachable from a published entry-point graph.

Tests must not assert an exact workspace package count, exact per-domain package-name list, or
path-based exception table.

## Invariants that must not change

The reorganization may break names, schemas, and persisted compatibility, but it must preserve:

- append-only stream families;
- per-append all-or-nothing store atomicity;
- content addressing of manifests, snapshots, facts, artifacts, and outputs;
- canonical JSON hashing without floats;
- deterministic operation expansion;
- no ambient IO in state logic;
- evidence-only replay;
- exact shared-snapshot semantics for batched fact queries;
- no secrets or bearer mutation material in persisted surfaces, logs, outputs, or errors;
- secret zeroization and keystore tamper protections; and
- thin binaries and reusable libraries.

## Compatibility and persisted data

This RFC intentionally provides no backward compatibility.

Package consolidation and type removal may change capability IDs, descriptor IDs, schema IDs,
certified spec digests, and run manifests. Old runs, fixtures, and databases are not migrated or read
through compatibility adapters. Development and test databases should be recreated when the new
authority model lands.

The implementation must fail clearly if old retained data is presented. It must not guess, silently
reinterpret, or fall back to old schemas. Code needed only for the old topology is deleted and
remains recoverable from Git history.

## Commit plan

Implementation is split into independently reviewable, buildable commits. Commit subjects are lower
case and describe one logical change:

1. `replace topology snapshots with boundary contracts`
2. `delete unused proof workflow`
3. `delete btc checkpoint collector surface`
4. `delete evm standalone collector cycle`
5. `limit production registries to published graphs`
6. `fold artifact reads into store`
7. `add generic pure state runner`
8. `add generic managed fact runner`
9. `fold fact contracts into facts`
10. `fold effects into capabilities`
11. `collapse bitcoin collection to one read and one write`
12. `consolidate bitcoin domain and live crates`
13. `consolidate evm domain and live crates`
14. `consolidate portfolio domain and live crates`
15. `merge keystore implementation and provider`

Each commit must:

- delete the superseded packages, types, tests, docs, and wiring in the same commit;
- avoid transitional re-exports or dual implementations;
- update architecture/design documentation for its completed state;
- use focused Cargo checks during development; and
- pass `nix run .#check`, `nix run .#test`, and `nix run .#test-db` before commit.

After the final commit, run `nix run .#ci` and require clean `git status --short` output alongside the
retained `closing-source-revision` evidence.

## Acceptance criteria

The reorganization is complete when:

- the workspace follows the proposed pure-domain/live-domain structure;
- no top-level domain `*-capabilities` package remains;
- proof and standalone collector-cycle production surfaces are absent;
- the app registry is derived from published graph requirements rather than workspace inventory;
- ordinary pure and managed-fact-write states use one kernel execution/replay path each;
- the active Bitcoin collector has one aggregate read and one atomic fact-write state;
- no compatibility package, alias, fallback, or legacy schema reader remains;
- architecture tests enforce dependency and reachability properties without package counts;
- portfolio snapshot and explicit transaction-signing behavior remain covered by boundary tests;
- keystore security and redaction tests pass after consolidation;
- all required Nix gates and final CI pass; and
- the worktree is clean.

## Alternatives rejected

### Keep the current packages and improve naming

Rejected because naming does not reduce dependency fan-out, duplicated runners, public boundary
types, or the number of places a domain change must touch.

### Merge the entire workspace into one library

Rejected because it removes valuable compile-time firebreaks between kernel, pure domain code, live
IO, secret-bearing providers, storage implementations, and binaries.

### Create one global live crate

Rejected because it permits unrelated protocol wiring to depend on and accumulate around each
other. Per-domain live crates enforce a useful boundary at low conceptual cost.

### Keep compatibility shell packages

Rejected because shells preserve old names, dependency paths, public APIs, and maintenance burden
without preserving useful architecture. Git history is the recovery mechanism.

### Add missing generic runners but retain adapter copies

Rejected because two normal execution paths would remain. Generic support is introduced only with a
complete consumer migration and deletion of the duplicated paths.
