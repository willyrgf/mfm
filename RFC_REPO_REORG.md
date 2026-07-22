# RFC: Repository Reorganization

- Status: Accepted
- Date: 2026-07-21
- Scope: Workspace package boundaries, execution paths, production assembly, and architecture enforcement

## Summary

MFM currently has 44 workspace packages. Many packages represent architectural roles such as
capability, state, operation, adapter, or transport rather than an independently reusable
abstraction or a necessary dependency firebreak. The result is a wide dependency graph, public
types that exist mainly to cross package boundaries, repeated runner and replay mechanics, and
architecture tests that preserve the current inventory instead of the intended properties.

This RFC adopts a more concentrated package structure as the current design:

- one pure crate for each active domain: Bitcoin, EVM, and portfolio;
- one live-integration crate for each domain that needs process-local wiring;
- a small set of independently reusable kernel, storage, signing, and secret-bearing crates; and
- one app assembly crate above them.

Capability, model, signing, state, operation, adapter, and transport remain distinct
responsibilities. They stop being automatic reasons to create crates. Internal module direction,
Rust visibility, tests, and the remaining Cargo firebreaks preserve the separation of concerns.

The one-pure-crate-per-domain layout is a deliberate concentration experiment, not an eternal
package-count rule. If a future domain becomes large enough that Cargo must enforce a dependency
prohibition which modules and tests cannot enforce clearly, that specific boundary can be proposed
from evidence.

The RFC also removes an unnecessary execution concept. Facts produced deterministically from an
external read will be returned by that read state and committed atomically with its evidence,
output, and completion. MFM will not add a generic managed-fact-write state or runner.

## Decision principles

This reorganization applies these priorities in order:

1. Fewer concepts.
2. Fewer execution and replay paths.
3. Fewer public types and schemas.
4. Fewer duplicated responsibilities.
5. Fewer places a future change must touch.
6. Fewer lines of code after correctness, security, and replay guarantees are preserved.

There is no backward-compatibility requirement. A replacement deletes the replaced package, type,
schema, implementation, fixture, and documentation in the same logical change. The implementation
must not create compatibility crates, deprecated aliases, re-export facades, alternate legacy
readers, feature-gated old paths, or fallbacks. Git history is the recovery mechanism.

A crate is justified when at least one of the following is true:

- Cargo must enforce a dependency prohibition that cannot be kept clear with modules, visibility,
  and architecture tests;
- it is an independently reusable platform primitive with a coherent public contract; or
- it contains proc-macro, storage implementation, process assembly, or secret-bearing code whose
  isolation is itself a security or build boundary.

Repository taxonomy alone is not sufficient justification.

## Material uncertainties

Implementation evidence resolved five of the six empirical assumptions: the concentrated domain
boundaries have source-role and public-API guardrails; memory and Postgres exercise uncertain
external-read settlement without repeated live IO; catalog generation and representative branch
coverage agree; the transport passes the pinned Bitcoin Core parity task; and adversarial JSON/TOML
tests close selected and unselected secret-source shapes.

One per-revision verification dependency remains until the final revision is published:

- **Choice or assumption:** the target-specific executable reader hashes the immutable packaged
  executable path correctly on macOS, while evidence-only services remain independent of current-
  executable IO.
- **Why it is uncertain:** the local development host is Linux; only the supported hosted macOS
  workflow can exercise the packaged-path implementation on the exact final Git revision.
- **Consequence if wrong:** executable identity would not be validated on both supported targets,
  so the reorganization could not satisfy its acceptance criteria.
- **Resolution:** push the locally verified final revision and require the macOS `ci-full` job in
  `.github/workflows/checks.yml` to pass for that exact Git SHA. A failure requires a new execution-
  identity decision, not a label fallback.

No backward-compatibility, migration, provider-ownership, or transport-visibility decision remains
open.

## Problem situation

### Architectural taxonomy became package topology

The repository distinguishes domain models, capabilities, states, operations, adapters,
transports, signers, storage, and app assembly. Those distinctions are useful for assigning
responsibility, but the workspace commonly assigns each role its own crate.

The EVM balance path illustrates the cost: capability contracts, signing, states, operations,
adapters, and transports are six crates even though they form one bounded domain used by one
product assembly. Bitcoin and portfolio follow similar patterns. Ordinary domain changes therefore
cross several manifests and public APIs, and types become public only so neighboring crates can
exchange them.

### Capability crates do not represent one abstraction

The top-level `*-capabilities` crates have unrelated ownership:

- `mfm-artifact-capabilities` wraps store-owned retained-artifact authority and duplicates artifact
  evidence shapes;
- `mfm-fact-capabilities` mixes domain-free fact semantics with store-backed query ports;
- `mfm-btc-capabilities` and `mfm-evm-capabilities` contain protocol-domain requests, identities,
  evidence, errors, and provider contracts; and
- `mfm-capabilities` is the kernel authority model and re-exports `mfm-effects`.

The suffix does not describe a useful shared boundary. It scatters domain code while separating
kernel concepts that already change together.

### Runner and replay infrastructure is incomplete

The runtime has a generic external-read path, but adapters repeat ordinary pure-state execution,
configuration and input loading, output construction, and replay verification. Bitcoin and EVM
also use a second managed-write path merely to persist facts already determined by the immediately
preceding external read.

The correct simplification is not another adapter package or a new generic managed-write runner.
It is:

- one generic pure-state execution/replay path; and
- one external-read settlement path that may atomically include the facts returned by the read.

### Facts and artifacts have platform-wide ownership

Facts and retained artifacts are not portfolio concepts. Bitcoin and EVM produce them today, and
future domains will produce and consume more of them. Assigning their provider implementations to
`mfm-portfolio-live` would create a false ownership boundary and make every new fact-producing
domain touch portfolio code.

Domain-free fact semantics belong in `mfm-facts`. Store IO, projections, retained artifact reads,
and concrete provider implementations belong at the store boundary. `mfm-portfolio-live` owns only
portfolio-specific query orchestration and hydration.

### Production assembly contains disconnected topology

The app publishes one run entry point: `mfm.portfolio/snapshot@1`. Production registries also
contain descriptors and runners not reachable from that graph, including:

- the proof workflow;
- the standalone Bitcoin chain-head/checkpoint collector cycle;
- the standalone EVM balance-cycle launch wrapper; and
- reusable EVM transaction and exact-anchor validation foundations with no published operation.

Reusable library code does not need automatic production registration. The proof workflow,
Bitcoin checkpoint cycle, and EVM standalone wrapper have no product owner and will be deleted.
The EVM transaction and validation foundations remain reusable library APIs but will not enter the
production registry until a published operation uses them.

### Architecture tests preserve accidents

The Cargo metadata contract currently asserts an exact package inventory, exact per-domain package
names, path-derived categories, and named exceptions. Those assertions make simplification appear
to be a violation even when the dependency graph improves.

Architecture tests must enforce dependency direction, source-role direction, public API shape, and
production authoring-catalog equality. They must not freeze a package count or directory spelling.

## Goals

- Test a concentrated one-pure-crate-per-domain design while keeping explicit responsibility
  boundaries.
- Eliminate top-level domain `*-capabilities` crates.
- Keep pure domain behavior reusable without app, binaries, live IO, keystore, or concrete storage.
- Keep Bitcoin and EVM transports independently reusable without their runtime adapters.
- Centralize ordinary pure-state and external-read execution/replay paths.
- Make facts and retained artifacts platform services rather than portfolio implementations.
- Remove dormant workflows, registrations, schemas, and types.
- Preserve thin binaries without manufacturing app-layer mirror types.
- Preserve all security, canonicalization, append atomicity, replay, and no-ambient-IO invariants.

## Non-goals

- Creating one monolithic MFM library.
- Merging pure domain logic with live IO.
- Treating adapter and transport as one responsibility merely because they share a crate.
- Merging generic signing contracts with the secret-bearing keystore implementation.
- Merging store contracts with the Postgres implementation.
- Merging the proc-macro crate into a runtime library.
- Adding speculative fact scopes, provider layers, entry points, workflow modes, or protocol
  fallbacks.
- Reading or migrating data created by the deleted architecture.

## Target package structure

```text
crates/
|-- kernel/                         # independently reusable platform contracts
|   |-- ids
|   |-- canonical
|   |-- values
|   |-- capabilities               # absorbs surviving effect contracts
|   |-- facts                      # domain-free fact semantics and query capability
|   |-- program
|   |-- program-derive
|   |-- spec
|   |-- certify
|   |-- events
|   |-- store                      # store traits, fact query IO, retained artifact IO
|   |-- manual-auth
|   |-- runtime
|   `-- replay
|-- domains/
|   |-- bitcoin                    # model, capability, state, operation
|   |-- evm                        # model, capability, signing, state, operation
|   `-- portfolio                  # model, state, operation
|-- live/
|   |-- bitcoin                    # public transport, private adapter/registration
|   |-- evm                        # public transport, private adapter/registration
|   `-- portfolio                  # portfolio query/hydration and registration only
|-- signing                        # generic signer contract
|-- keystore                       # secret storage and signing implementation
|-- storages/
|   `-- postgres                   # concrete store and fact/artifact providers
`-- app                            # process config, assembly, and application services

bin/
|-- cli
`-- rest-api

tests/
`-- integration
```

This currently projects to approximately 27 workspace crates. The number is an outcome and must
not be asserted by tests. Future consolidation or separation is valid only when it improves the
properties in this RFC.

### Pure domain crate contract

Each pure domain crate uses private role modules and exposes only the domain API required by its
consumers:

```text
mfm-bitcoin
|-- model
|-- capability
|-- state
`-- operation

mfm-evm
|-- model
|-- capability
|-- signing
|-- state
`-- operation

mfm-portfolio
|-- model
|-- state
`-- operation
```

The allowed source direction is:

```text
model  <-  capability  <-  signing  <-  state  <-  operation
```

`model` imports no other domain role. `capability` may import `model`. `signing` may import `model`
and `capability`. `state` may import every lower role, and `operation` may import every lower role.
No role imports a role above it. A domain without a role simply skips it. Domain root modules
selectively re-export the smallest useful API; role modules are not made public just to reproduce
the old crate paths.

No pure domain module may depend on runtime execution, replay, app assembly, live integration,
concrete stores, keystore, filesystem, or network clients. Operations remain deterministic graph
construction; states remain reusable domain semantics with explicit capabilities.

### Live integration crate contract

Bitcoin and EVM live crates colocate transport and adapter code but preserve a hard responsibility
boundary:

```text
mfm-<domain>-live
|-- pub mod transport
`-- mod adapter
```

The canonical transport API is `mfm_<domain>_live::transport::*`. It is not also glob-re-exported
from the crate root. The public transport module exposes a checked, typed protocol session,
resolved endpoint/authentication inputs, and redacted typed errors. Raw HTTP/JSON-RPC envelopes and
an arbitrary public `rpc_call` escape hatch remain private.

Transport code implements pure-domain session/provider traits and must be usable without creating a
runtime runner registry. It must not name adapters, runner identities, runtime, replay, or stores.
Transport and adapter are sibling modules which both depend on the pure-domain session trait; the
adapter must not import the concrete transport. App assembly constructs the public transport
session and passes it to the narrow root registration function. Adapter tests pass a fake session.

Configured routes are instances, not capability implementations. For each start or resume,
`mfm-app` extracts the distinct route keys required by pending certified live nodes, resolves only
those entries, constructs one public endpoint-bound session per key, and places the sessions in one
app-private routed set. Bitcoin keys are `semantic_source_identity`; EVM keys are
`(network_id, source_ref)`. The routed set implements the same pure-domain session trait as a
standalone transport session, is passed through one `Arc`, self-reports the one implementation ID,
and is registered once. Missing, duplicate, or mismatched route bindings fail before run admission.

Colocation removes manifests and cross-crate plumbing. It does not merge the two responsibilities.
Independent reuse is an API and source-dependency property, not a feature split: the live crate is
built as one crate. Do not add adapter/transport feature flags or a second transport package to
simulate separate compilation.

`mfm-portfolio-live` has no transport and owns no generic fact or artifact provider. It owns the
portfolio-specific asynchronous work of querying facts, reading the exact retained response
artifacts selected by those facts, decoding them through the pure Bitcoin/EVM domain APIs, and
building portfolio query evidence. Registration accepts one shared store value implementing both
fact-query and retained-artifact-read traits. This prevents assembly from accidentally supplying
separate provider objects; retained artifact identity and bytes are still independently verified.

## Package disposition

| Current crates | Disposition |
|---|---|
| `mfm-effects` | Move only surviving effect contracts into `mfm-capabilities`; delete the crate. |
| `mfm-fact-capabilities` | Move fact semantics/query capability to `mfm-facts` and async store IO to `mfm-store`; delete the crate. |
| `mfm-artifact-capabilities` | Move retained-artifact authority and evidence validation to `mfm-store`; delete the crate and app facade. |
| `mfm-btc-capabilities`, `mfm-states-btc`, `mfm-op-btc-collectors` | Consolidate as `mfm-bitcoin`. |
| `mfm-adapters-btc-jsonrpc`, `mfm-transports-btc-jsonrpc-http` | Consolidate as `mfm-bitcoin-live`, with public transport and private adapter modules. |
| `mfm-evm-capabilities`, `mfm-evm-signing`, `mfm-states-evm`, `mfm-op-evm-collectors` | Consolidate as `mfm-evm`. |
| `mfm-adapters-evm`, `mfm-transports-evm` | Consolidate as `mfm-evm-live`, with public transport and private adapter modules. |
| `mfm-portfolio-model`, `mfm-state-portfolio`, `mfm-op-portfolio-snapshot` | Consolidate as `mfm-portfolio`. |
| `mfm-adapters-portfolio` | Becomes `mfm-portfolio-live`; generic provider implementations are removed from it. |
| `mfm-runtime-config` | Move process configuration parsing/resolution to `mfm-app`; delete the crate. |
| `mfm_core`, `mfm-signers-keystore` | In one cut, delete secret export/raw-key bridges and consolidate as `mfm-keystore`. |
| `mfm-collectors-proof`, `mfm-op-proof`, `mfm-transports-proof` | Delete completely. |

Crates not listed remain separate unless another evidence-backed change shows their firebreak is
unnecessary.

## Execution-path consolidation

### Generic pure-state execution and replay

`mfm-runtime` will own one generic pure-state runner. It performs the standard typed sequence:

1. load and validate certified configuration, inputs, and context;
2. invoke deterministic state behavior;
3. build canonical outputs and artifacts; and
4. return the runtime-owned settlement.

`mfm-replay` will use the matching generic evidence-only replay path. Every adapter copy of this
sequence is migrated and deleted in the introducing change.

The runner receives a caller-supplied factory binding; runtime never invents process provenance.
At live-process assembly, `mfm-app` streams the opened current executable through raw SHA-256 once,
places that lower-case digest in one domain-separated canonical JSON identity object, constructs one
`ExecutableIdentityTemplate` from the object's content digest, and uses the template for framework
and domain runner factories. `ExecutableIdentity` contains only `factory_id` and `binary_digest`;
redundant package and unpopulated Nix provenance fields/types are deleted in the same
persisted-contract cut. The old runner-specific template name and every label-derived
`binary_digest` path are deleted. Standalone library assemblers must provide an explicit checked
template/binding; there is no runtime default or fallback. This keeps generic runner code reusable
while making the persisted binary digest bind executable bytes rather than a stable label.

This is intentionally fail-closed: different CLI and REST executables, or a rebuilt executable,
cannot live-resume one another's admitted run even when they contain the same app library.
Evidence-only replay validates recorded evidence and performs no current-executable IO; read-only
status/stream access is likewise unaffected. MFM does not promise old-run live execution across
builds, and it does not weaken the digest into a shared label to gain that compatibility.

### External reads emit their deterministic fact batch

The existing external-read state contract will gain one sealed associated fact batch:

```rust
trait ReadState {
    type Facts: ReadFactBatch;

    fn reduce(/* checked request and evidence */)
        -> StateResult<(Self::Output, Self::Facts)>;
}
```

`ReadFactBatch` is a public-but-sealed framework trait in `mfm-program`, because it appears in the
public `ReadState` bound. It exposes only
`fn fact_descriptor() -> mfm_facts::Result<Option<FactDescriptor>>` and has exactly two framework
implementations:

- `()`, returning `Ok(None)`, for an external read that emits no facts; and
- `NonEmpty<F>`, returning `F::descriptor().map(Some)`, for a homogeneous, non-empty batch where
  `F: MfmFactType`.

Domains cannot create alternate settlement behaviors. Runtime and replay use private companion
traits for staging and comparison of those two concrete batch forms; there is no public erased
fact, visitor, or extension hook. `MfmFactType` moves from `mfm-program` to `mfm-facts`.

The external-read effect contract uses one private version-2 digest over Plan, Evidence, and Facts.
It binds plan identity, evidence identity, and canonical fact mode: either `none`, or `non_empty`
plus the emitted fact descriptor hash. The effect runner derives emitted descriptors; manual
state/fact registration hooks and the version-1 digest path are deleted.

For a successful external read, the semantic payload order is `FactRecorded*`, `CellProduced`, then
`StateAttemptCompleted`. Primary and optional query evidence artifacts, artifact admissions,
derived artifact-reference events, and retention records join those payloads in the same
all-or-nothing settlement append. `FactRecorded` and `ArtifactReferenced` are not terminal attempt
dispositions. The attempt-start append may already be visible; after an uncertain settlement result,
runtime reloads and verifies the stream before it performs live IO again. It never blindly retries
an uncertain append. Replay invokes the same reducer and compares exact output and fact batch.

This replaces and deletes:

- `ManagedWriteState`, `ManagedPlatformWrite`, and the proposed `ManagedFactWrite` concept;
- managed-write runner, effect, role, certification, recovery, and replay branches;
- `FactRecordCapability` and domain adapter record bindings;
- Bitcoin/EVM fact-record states and their intermediate-only schemas; and
- fact-prefix recovery such as `RecordedFacts`.

There is one fact-producing read path, not a read path followed by a generic write path.

## Fact and artifact ownership

`mfm-facts` owns domain-free fact semantics: fact type and descriptor contracts, canonical hashing,
content identities, claims, references, query plans/results/evidence, and the state-facing
`FactQueryReadCapability`.

`mfm-store` owns asynchronous least-authority IO and projection mechanics:

- `FactQueryStore`;
- the shared snapshot/frontier contract, represented only by the actual store-owned `StoreScopeId`
  and store-wide `StoreCommitOrder`;
- fact projections and rebuild;
- `RetainedArtifactReadProvider`;
- `EventArtifactRequirement`, `EventArtifactReferenceSource`, and event-to-requirement construction;
- verified retained artifact bytes; and
- evidence validation against store events.

`PostgresStore` and `AsyncInMemoryRunStore` implement the query and retained-artifact traits
directly, with identities `mfm.storage.postgres.fact-query.v1` and
`mfm.store.memory.fact-query.v1`. Provider identities do not use `mfm.app.*`.
`mfm-app` owns application services and public presentation DTOs, not generic storage providers.

The current generic fact surface contains speculative distinctions with no production producer or
consumer. This reorganization deletes `FactProducerProvenance`, generic `request` and
`observed_at` claim/metadata fields, Control/RunPrivate visibility and query-scope variants,
`StoreScopeRef`, `ScopeDecisionEvidence`, and the duplicate record-only fact projection.
`recorded_at` remains store-envelope authority and the real `StoreScopeId` remains run/store
authority. Protocol observation/source semantics remain in typed response evidence. The current
public facts query means platform facts; a future private/control fact requires a concrete access
model.

Every descriptor admission and fact append advances `StoreCommitOrder`, so the scope/order pair
identifies the complete append-only query snapshot. The old descriptor-count watermark and
Prefix/Snapshot frontier variants are deleted; neither adds authority.

Adding a future fact-producing domain therefore changes that domain and the appropriate published
authoring catalog, while using the unchanged generic external-read/fact registration mechanism. It
does not require portfolio code or another platform registration hook.

## Bitcoin collection simplification

The portfolio Bitcoin collection becomes one state:

```text
BitcoinBalanceCollectionOperation
  -> CollectBitcoinBalancesState   # external evidence + facts + receipt
```

One successful attempt performs exactly three logical Bitcoin Core RPC calls:

1. `getblockchaininfo` to validate the checked chain/source;
2. one multi-descriptor `scantxoutset start` covering the complete strictly sorted address set; and
3. `getblockhash(scan.height)` to bind the scan result to its exact canonical block hash.

The state validates the complete response, deterministically reduces per-address totals, returns a
non-empty balance-fact batch and collection receipt, and lets the external-read runner settle all
of it atomically. The supported request limit remains 1,024 addresses and the complete success or
error response is capped at 16 MiB. Canonical rendered address UTF-8 byte order is the single order
used by requests, descriptors, evidence, facts, and receipts.

The transport does not implement hidden retries. Runtime node retry repeats the complete attempt.
MFM also does not add a process-local scan coordinator: it could not enforce exclusion across
processes and would imply a guarantee it cannot provide. A Bitcoin Core scan-busy response becomes
a redacted retriable operational error. MFM never issues `status` or `abort` against work it does
not own. The selected route supplies a bounded scan deadline. A timeout or cancellation drops MFM's
request but does not claim to abort Bitcoin Core's scan; a later retry may therefore receive the
same scan-busy response. The client disables redirects and ambient proxy discovery so authentication
and the three-request boundary remain tied to the selected endpoint.

The decoder supports the common Bitcoin Core 28+ response fields MFM uses, strictly validates their
types and semantics, and permits additive unknown response fields. Core 28 is the minimum because
it is the first release with both JSON-RPC 2.0 and testnet4. MFM sends and accepts only JSON-RPC 2.0;
there is no legacy 1.1 path. The bounded parser rejects duplicate JSON members at every object level
before typed decoding; unique unknown members remain forward-additive. That is one forward-additive
decoder, not version negotiation or a fallback. `rust-bitcoin` is authoritative for address
parsing/canonical rendering, address-network encoding compatibility, script derivation, block
hashes, transaction IDs, outpoints, and checked satoshi values. The checked RPC chain tag
establishes the actual chain because test-family address encodings do not uniquely distinguish
testnet, testnet4, signet, and regtest. MFM constructs fixed `addr(...)` scan descriptors and does
not interpret Bitcoin Core's returned descriptor strings.

The replacement identities are one deliberate incompatible cut. The operation is
`mfm.bitcoin.balance_collection`, kind `(mfm.bitcoin, balance_collection)`, digest material
`mfm.bitcoin.operation:balance_collection`, version
`mfm.bitcoin.operation.balance_collection.v1`. The state is `mfm.bitcoin.collect_balances`, kind
`(mfm.bitcoin, collect_balances)`, digest material `mfm.bitcoin.state:collect_balances`, version
`mfm.bitcoin.state.collect_balances.v1`. The Bitcoin JSON-RPC adapter keeps its kind/digest and
becomes `mfm.bitcoin.jsonrpc.adapter.v2`. Its new fact kind is `bitcoin.balance_snapshot`.

The subject, response, fact, config, plan, evidence, receipt, and operation-output schemas are,
respectively, `mfm.bitcoin.fact.balance_snapshot.subject`,
`mfm.bitcoin.fact.balance_snapshot.response`, `mfm.bitcoin.fact.balance_snapshot`,
`mfm.bitcoin.balance_collection.config`, `mfm.bitcoin.balance_collection.plan`,
`mfm.bitcoin.balance_collection.evidence`, `mfm.bitcoin.balance_collection.receipt`, and
`mfm.bitcoin.operation_outputs.balance_collection`, all at semantic version 1. EVM and portfolio
also replace their behavior bindings in the same persisted-contract cut with
`mfm.evm.jsonrpc.adapter.v2` and `mfm.portfolio.adapter.typed.v2`; no adapter-v1 identity remains
accepted.

The standalone checkpoint/chain-head fact types, states, cycle operation, launch helpers, and
receipt assembly are deleted. Chain-head resolution remains an internal part of the aggregate read.

## Production registration

The sealed authoring catalog declared by `mfm.portfolio/snapshot@2` is the production universe for
possible domain descriptors. This is a declared catalog, not a claim that static metadata proves
semantic reachability. It is defined independently of app registry construction and lists the
operation's allowed child operations, states, capability descriptors, and facts. It does not and
cannot name process/build executables, configured routes, store implementations, or live adapter
implementation IDs. Production certification and emitted-fact descriptor sets must equal that
semantic catalog plus one sealed `mfm-runtime` bootstrap containing framework primitives only.
Runner, capability-binding, executable, side-effect-verifier, and replay registries must have
exactly the corresponding catalog keys, with one concrete binding per required key and no extra
domain key.

The catalog and typed authoring/certification registries are generated from one state, operation,
and include declaration. The typed builder rejects an attempted child whose descriptor is absent
from that generated catalog, proving emitted graphs are a subset. The union of the canonical
Bitcoin-only, EVM-only, and mixed expansions must equal the declared domain operation/state sets,
proving that every declared item has a valid published branch. App assembly is never used to derive
either side of this equality.

BTC-only, EVM-only, and mixed drafts remain behavioral smoke tests and collectively exercise each
declared domain item, but they do not define catalog completeness. Every declared state has exactly
one runner binding and every declared capability has exactly one implementation binding in a
process. Concrete IDs come from the selected store/session and are validated against the same
object used for execution; configured routes are instances, not catalog entries.

Under this rule:

- portfolio Bitcoin and EVM collection are registered through the portfolio graph;
- proof and standalone collector cycles disappear; and
- reusable EVM transaction and exact-anchor validation APIs remain library-testable but absent from
  production registries until a published graph consumes them.

The public snapshot removes the derived constant `coverage` field, so
`PortfolioSnapshot::SCHEMA_VERSION` becomes 2 and `mfm.portfolio/snapshot@2` replaces @1 as the sole
entry point. The report shape remains schema version 1. There is no @1 alias or dual registration.

## Runtime configuration and secret boundary

`mfm-app` owns bounded process configuration parsing and resolution. Live crates accept resolved,
typed endpoint/authentication inputs; they do not open config files or read environment variables.
Evidence-only execution never resolves current configuration.

The configuration path is supplied explicitly to app services. CLI and the REST server expose only
`--runtime-config <PATH>`; the old environment-selected path and precedence rule are deleted.
Absence is valid for evidence-only work and fails only when pending live execution needs it.

The configuration document has four final top-level sections: `bitcoin`, `evm`, `signers`, and
`keystores`. The parser:

- parses the complete bounded JSON or TOML syntax;
- rejects duplicate JSON keys at every level;
- rejects unknown top-level sections; outside the two reviewed secret slots, rejects normalized
  keys containing the closed `password`, `passphrase`, `mnemonic`, `seed_phrase`, `private_key`,
  `secret`, `credential`, `api_key`, `access_key`, `token`, `authorization`, `signed_material`,
  `signed_transaction`, `raw_transaction`, `raw_tx`, or `signature_scalar` marker; and rejects a
  direct value in
  `bitcoin.routes.*.rpc_password` and `evm.routes.*.auth_header`, including in unselected entries;
  and
- performs strict schema and semantic deserialization only for the selected entry set and referenced
  keystores, so an unrelated unused entry does not block a run.

A source value uses one tagged choice with exactly one key:

```text
{ direct = ... } | { env = ... } | { file = ... } | { file_env = ... }
```

Direct values are prohibited for the reviewed secret slots. Keystore configuration contains an
unlock-file path, never an unlock secret. Establish a 1 MiB document ceiling and a 64 KiB selected
resolved-value/indirection-file ceiling as supported resource limits. Each Bitcoin route requires
`scan_timeout_seconds` in `1..=86400`; app passes the resulting duration to the transport for the
single `scantxoutset` request. Error surfaces redact paths, environment names, and values.
Bitcoin/EVM live crates own consuming, zeroizing, non-serializable, redacted authentication input
types; app resolution moves selected secrets into them and retains no second copy. Secret file and
environment values enter zeroizing byte storage immediately, use limit-plus-one reads, and become a
zeroizing string without an ordinary secret buffer on success or failure.

The keystore consolidation atomically deletes the unused `dangerous-secret-export` feature,
`allow_secret_exports`, raw private-key export APIs, their audit/error surface, and the cross-crate
raw-key bridge. Raw key access becomes crate-private in that same change. The only binary-to-app
secret input is one non-cloneable, non-debuggable, non-serializable zeroizing app type consumed by
the app service. Runtime signing stores only a redacted unlock-file path; each signing call uses a
zeroizing limit-plus-one buffer on a blocking worker, rejects oversize rather than truncating,
strips at most one terminal line ending, and zeroizes every success/error path.

## Binary boundary

CLI and REST binaries parse transport input, call application services, and render output. They do
not depend directly on runtime configuration, keystore implementation, live integrations, runtime,
replay, concrete storage, signing implementations, or domain execution behavior.

They may use explicitly presentation-safe kernel value contracts such as canonical bytes and typed
IDs when those types are genuinely part of binary input/output. `mfm-app` must not introduce a DTO
or trait whose only purpose is to mirror such a lower-level type. App assembly remains the only
execution/implementation dependency.

## Architecture enforcement

Cargo metadata records semantic layer, domain, domain role, kernel domain-facing status, and
binary-facing status for kernel/assembly packages, not a frozen directory inventory. Domain IDs are
validated but not hardcoded. Only `mfm-app`, `mfm-ids`, and `mfm-canonical` are binary-facing.
Cargo target kinds constrain those declarations: any package with a binary target is layer
`binary`, and a mixed lib/bin package must obey the binary dependency row for the whole package.
Proc-macro targets remain dedicated non-binary packages.
Architecture tests enforce:

1. kernel dependency direction;
2. pure domains never depend on app, binaries, live crates, runtime/replay execution, keystore, or
   concrete stores;
3. live crates depend only on their corresponding pure domain plus platform contracts, except that
   portfolio live may consume pure Bitcoin/EVM decoding contracts;
4. Bitcoin/EVM transport modules are public and reusable without adapters, while adapter modules
   remain private;
5. source-role direction inside concentrated crates and sibling transport/adapter dependence on the
   same pure session trait;
6. generic signing never depends on keystore and store contracts never depend on Postgres;
7. binaries obey the thin boundary without forcing presentation mirror types;
8. the proc-macro crate remains separate; and
9. production registration equals the published operation's declared authoring catalog.

Source scans are guardrails, not the sole proof. Rust visibility/API tests and positive compile/use
tests establish that a typed transport can be used without registry construction and an adapter can
be registered against a fake session without concrete HTTP.

Tests must not assert an exact package count, exact domain-package list, or exception table. The
existing source-text assertions around Bitcoin transport methods are replaced with API/privacy
tests: the checked typed session is public, while raw JSON-RPC bypass methods remain private.

## Invariants that must not change

The reorganization may replace every old name and persisted schema, but it preserves:

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

## Persisted-data cut

There is one current schema and authority model after each change. Changed content-addressed
structures naturally produce changed identities and digests. Strict current decoders reject
deleted or unknown fields where the schema is closed.

The Postgres baseline and authority fingerprint are reset when the fact/event/store cut lands.
Store connection authority mismatch and migration checksum mismatch map to one public redacted
`IncompatibleStoreSchema` error, while content-addressed domain contract changes reject old streams
through ordinary certification. An old or modified store must be recreated. There is no migration,
compatibility reader, version negotiation, legacy fixture, or fallback path.

That persisted cut also reduces `ExecutableIdentity` to `factory_id` plus the exact
`binary_digest`; package-label and unpopulated Nix fields/types are removed from events, codecs,
schemas, and replay evidence. The one shared event schema version becomes 2 for all retained
`mfm.events.v1.*` names, avoiding per-event version branches while the changed FactRecorded and
RunAdmitted shapes land together. The later runner-centralization commit changes the producer of
the surviving digest from labels to current-executable bytes without creating another persisted
shape.

## Delivery strategy

Implementation is divided into dependency-ordered, independently reviewable logical changes. The
detailed scope, tests, deletion ledger, and as-built commit mapping live in
`IMPL_PLAN_RFC_REPO_REORG.md`. The ordered delivery phases are:

1. install final semantic metadata and delete unused proof/checkpoint/standalone registration;
2. move retained-artifact authority to the store;
3. make rust-bitcoin the active Bitcoin primitive authority;
4. atomically replace fact-query ownership, managed fact writes, the old fact/store schema, and
   per-address Bitcoin collection with one query authority, one external-read fact settlement, and
   one aggregate Bitcoin read;
5. fold effects into capabilities, bind live registries to exact executable bytes, and add the
   generic pure-state execution/replay path;
6. consolidate final Bitcoin pure/live code;
7. prove the final public transport against pinned Bitcoin Core;
8. consolidate final EVM and portfolio pure/live code;
9. bind production to the final published authoring catalog;
10. delete secret export and consolidate keystore/configuration behind app services;
11. finish thin binaries; and
12. audit source-role, transport-reuse, authoring-catalog, and full-matrix guardrails.

The implementation plan divides these phases into numbered implementation units, not an exact Git
commit count. A unit may use multiple focused commits or later review remediations, provided every
commit leaves one coherent current design and every cutover deletes its superseded code in the same
logical change. Verification is scope-driven under `docs/build-and-verification.md`; broad gates
are not repeated per commit. The final cross-cutting revision runs `nix run .#ci` once.

## Acceptance criteria

The reorganization is complete when:

- Bitcoin, EVM, and portfolio each have one pure domain crate;
- Bitcoin and EVM transports remain public, typed, and usable independently of private adapters;
- `mfm-portfolio-live` contains no generic fact/artifact provider implementation or transport;
- no top-level domain `*-capabilities`, proof, standalone collector-cycle, managed-write, legacy
  config, or secret-export surface remains;
- ordinary pure states use one generic execution/replay path;
- fact-producing external reads use one atomic evidence/fact/output settlement path;
- live runner registries bind exact worker bytes through one template, while evidence-only replay
  does not inspect the current executable;
- the active Bitcoin collector is one aggregate read with exactly the specified three logical RPCs;
- production semantic keys equal the published operation's authoring catalog, with exactly one
  concrete binding for every required runner and capability key;
- no compatibility alias, facade, reader, migration, or fallback remains;
- architecture tests enforce semantic dependencies and source roles without package counts;
- portfolio snapshot, Bitcoin Core 28+/testnet4, EVM foundation, fact atomicity/replay, transport
  reuse, and keystore security behavior have boundary tests;
- the final required verification passes; and
- the worktree is clean.

## Alternatives rejected

### Keep the current crates and improve naming

Rejected because names do not reduce dependency fan-out, duplicate execution, public boundary
types, or the number of places a domain change must touch.

### Merge the entire workspace into one library

Rejected because kernel, pure domain, live IO, secret-bearing implementation, concrete storage,
proc-macro, and process assembly remain valuable Cargo firebreaks.

### Split every domain role into a crate

Rejected as the default because the current workspace demonstrates substantial manifest and public
API cost without evidence that each split prevents a real dependency violation. The concentrated
domain design is the chosen experiment.

### Make transports private inside live crates

Rejected because transports are reusable protocol primitives. Colocation with adapters must not
make independent transport construction or testing impossible.

### Put fact/artifact providers in portfolio live

Rejected because their semantics and implementations are platform/store-wide, and future domains
must not depend on portfolio ownership.

### Add a generic managed-fact-write runner

Rejected because current fact writes are deterministic products of external reads. Extending the
existing read settlement removes an entire state, runner, effect, recovery, and replay path.

### Keep compatibility shells or readers

Rejected because they preserve old names, schemas, dependency paths, and maintenance burden. Git
history provides recovery.
