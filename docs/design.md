# MFM Design Contract

Status: authoritative typed-core design contract

This document defines the runtime and authoring contract for the current MFM codebase. The typed
core RFC is now implemented enough that this document, not the removed dynamic model, is the
normative contributor-facing design reference.

The central rule is:

```text
typed state programs are the only semantic executable surface;
certified typed execution specs are the only runtime contract;
erased runner plans are implementation artifacts.
```

Operations may plan typed state programs, but operations are not runtime execution units after
certification. Binaries and app crates assemble stores, artifacts, registries, and capabilities;
they do not own workflow semantics.

## Non-Negotiable Invariants

- New execution uses certified typed execution specs only.
- Runtime state transitions are committed as typed kernel events through `mfm-store`.
- The typed run stream for `run:{run_id}` is append-only and authoritative.
- Store commits are atomic: a commit either appends every payload and updates derived projections,
  or appends nothing.
- Event envelopes, sequence numbers, ordinals, event ids, logical keys, and projections are
  store-owned.
- Manifest, config, seed, fact, artifact, output, and spec identities are content-addressed.
- Hashed structured data uses canonical JSON and must not contain floats.
- Typed values, typed configs, public outputs, and event payloads must not contain secrets.
- Replay and resume are driven by the stored certified spec and the authoritative run stream.
- Replay adapters must answer only from recorded facts, typed artifacts, and side-effect evidence.
- Side effects use typed intent, typed idempotency input, durable ledger events, and typed receipt
  or recovery evidence.
- Public output JSON is a render surface. Typed terminal cells plus public-output specs and events
  are the authority.
- Source scans, naming conventions, hash-only envelopes, persisted summaries, and CI summary keys
  are not typed-core architectural proof or runtime authority.

## Authority Surfaces

Typed-core code distinguishes data, evidence, authority, and implementation artifacts:

| Surface | Runtime authority? | Contract |
|---|---:|---|
| Parsed `TypedExecutionSpec` data | no | Persisted/user bytes decoded into typed Rust data. Hostile until certification verifies the spec against the registry. |
| `mfm_spec::v1::HashedSpecEnvelope` | no | Hash-only envelope for canonical spec bytes and non-semantic audit metadata. It cannot certify a spec. |
| `CertifiedSpecCertificate` bytes/evidence | no | Persisted certificate evidence. Hostile until the bundle verifier checks spec hash, certificate hash, registry digest, descriptor identities/digests, lowering/canonicalizer identity, public-output schema id, and audit metadata. |
| `mfm_certify::CertifiedTypedSpec` | yes | Non-forgeable in-memory authority minted only by registry-backed certification or verified persisted bundle input. |
| `mfm_runtime::CertifiedRuntimeSpec` | yes, runtime-only | Runtime wrapper derived only from `CertifiedTypedSpec`; owns scheduler indexes and erased runner derivation. |
| Erased runner plans | no | Runtime implementation artifacts reproducibly derived from certified authority and runner registry. |
| `PublicOutputReadAuthority` | yes, render-only | App authority minted after certified spec/certificate verification and projection rebuild from the authoritative stream. |
| Rendered public-output JSON/artifacts | no | Output/cache material for users and integrations. They cannot authorize resume, replay, or another render. |

Persisted spec bytes, persisted certificate bytes, rendered JSON, and projection rows must be
validated or rebuilt before they influence semantic execution.

## Crate Layout

Typed kernel crates are framework-owned and domain-free:

| Crate | Responsibility |
|---|---|
| `crates/kernel/ids` | Strong identity types for specs, states, events, values, artifacts, runs, and digests |
| `crates/kernel/canonical` | Canonical JSON bytes and content digests |
| `crates/kernel/values` | Typed value, config, artifact reference, and public-output descriptors |
| `crates/kernel/effects` | Framework-owned effect classes |
| `crates/kernel/capabilities` | Capability descriptors, roles, and effect-checked capability sets |
| `crates/kernel/program` | Typed state-program authoring, handles, scopes, registries, lineage, and lowering evidence |
| `crates/kernel/program-derive` | Derives for typed values, configs, state inputs, operation outputs, and public outputs |
| `crates/kernel/spec` | Versioned typed execution-spec data model and hash-only spec envelopes |
| `crates/kernel/certify` | Certification checks, non-forgeable certified typed-spec authority, and certificate bundles |
| `crates/kernel/events` | Versioned typed kernel event schemas |
| `crates/kernel/store` | Typed commit API, side-effect ledger rules, projection contract, and retention refs |
| `crates/kernel/runtime` | Certified typed scheduler and erased runner boundary |
| `crates/kernel/replay` | Replay authority, brokers, and verifier contracts |
| `crates/kernel/test-support` | Shared typed-kernel acceptance fixtures |

Domain and product crates sit outside the kernel:

| Area | Responsibility |
|---|---|
| `crates/states/*` | Typed state contracts and deterministic state-owned behavior |
| `crates/ops/*` | Typed operation planners that assemble state programs |
| `crates/transports/*` | Live and replay capability backends and erased typed runners |
| `crates/storages/*` | Implementations of typed store and typed artifact contracts |
| `crates/app` | Assembly of registries, stores, artifacts, start/resume/replay, and public output |
| `bin/cli`, `bin/rest-api` | Transport-only user surfaces |

Dependency direction is strict:

```text
ids -> canonical -> values -> effects/capabilities
  -> program/spec -> certify/events/store -> runtime/replay
```

Kernel crates must not depend on domain crates, binaries, app assembly, storage implementations, or
transport implementations. States must not depend on runtime/store implementations or binaries.
Ops may depend on typed states and domain config/model crates, but not on runtime scheduling or
storage implementations. Storages implement storage contracts and know no domain semantics.
Transports implement capability backends and runner bindings; they do not mint production authority
outside app/runtime assembly.

## Typed Values And Configs

Typed configs are deterministic planning inputs. They are decoded at boundaries, validated,
canonicalized, and retained by value or content-addressed reference in certified specs.
Run-start rejects config artifacts that match certified metadata but cannot be decoded and validated
through the trusted certification registry or an exact framework-owned config reference.

Typed runtime values cross state boundaries through typed cells and handles. A runtime value cannot
change the topology of the same certified run. If produced data must select future topology, the
workflow must create a new planning boundary such as a child run or a separately certified
continuation.

Persisted values and configs must implement the typed descriptor contracts through the framework
derive path or framework-owned generic constructors. Manual descriptor implementations outside the
framework boundary are rejected by checks because they bypass schema, no-float, and no-secret policy.

Secrets remain below the typed semantic boundary. Private keys, mnemonics, passwords, decrypted
bytes, raw signing material, and signed raw transactions must not be typed values, configs, facts,
artifacts, events, public outputs, error details, or fixtures. States refer to secret-bearing systems
through non-secret labels, references, and capabilities.

## Typed Program Authoring

State outputs are represented by branded typed handles. Handles carry the produced Rust value type,
program brand, scope brand, cell identity, schema identity, semantic type identity, and value
lineage. Handles cannot be forged from raw ids.

Scopes are part of certified semantics. Cross-scope same-value movement requires framework bridge
nodes with persisted bridge evidence. Transforming movement is modeled as an ordinary state.

Optional runtime paths use `MaybeValue<T>` cells, not absent context keys. Artifact references use
`ArtifactRef<T>` and must verify digest, schema id, semantic type id, role, and producer evidence
before materialization.

Operations are typed planners. They receive typed config and typed handles, assemble a typed state
program, bind public outputs, and return a draft for certification. Operation lineage and stable
domain keys are recorded in the certified spec so fanout/fanin ordering and semantic identity are
auditable.

## State And Operation Registration

A state type is executable only after framework registration validates:

- state kind and version
- config, input, output, and public descriptor identities
- effect class
- capability set
- side-effect contract when applicable
- runner kind and executable identity

Planning requires registered state or operation evidence. Runtime requires the certified descriptor
identity and the registered runner identity to match the stored spec.

## Effects And Capabilities

Effect classes are framework-owned:

- pure: no external capabilities
- read: read/support capabilities only
- managed platform write: platform persistence/output capabilities only
- apply side effect: exactly one external mutation authority plus allowed support capabilities

The runtime injects only capabilities certified for the current node. State code must not create its
own live network, filesystem, clock, process, or signer access when that access is part of semantic
execution. Domain helpers may compute deterministic values, parse data, or validate typed inputs,
but side effects and replayable observations must pass through typed capabilities.

## Certified Spec

`mfm-spec::v1::TypedExecutionSpec` is the persisted execution contract. It includes:

- spec version and canonicalization identity
- certified descriptors and executable identity requirements
- scopes, seeds, configs, nodes, cells, bridge nodes, and public outputs
- input binding trees and value lineage
- effect and capability evidence
- side-effect contracts
- retained config and seed artifact refs
- public-output render nodes and output evidence

The spec hash is computed from canonical bytes. The erased runner plan must be reproducibly derived
from the certified spec and runner registry. It must not carry semantics missing from the certified
spec.

`mfm_spec::v1::HashedSpecEnvelope` is not certification authority. Persisted spec bytes, hash-only
envelopes, and persisted certificate bytes are hostile data until `mfm-certify` verifies them
against a registry and returns `CertifiedTypedSpec`.

## Store And Events

`mfm-store` is the only semantic commit contract for certified typed runs. Production execution
callers submit `PreparedTypedCommit`, which carries typed event payloads, commit preconditions, and
the artifact evidence that becomes run authority in the same atomic append. The store constructs
envelopes and maintains projections. Synthetic direct mutation is confined to explicitly named
non-execution test, migration, repair, corruption, or low-level storage contract fixtures.

The authoritative event stream contains:

- run start and attempt lifecycle events
- seed/config/fact/artifact evidence
- cell terminal events
- side-effect ledger events
- public-output render and produced events
- retention refs and manifests
- run completion or terminal failure

Projection corruption is repairable by rebuilding from the run stream. Projection data must never
be the sole authority for resume, replay, public output, retention, or side-effect status.

The first certified persistent storage path is:

```text
crates/storages/stream-store-postgres + crates/storages/artifact-store-fs
```

Postgres stores typed event envelopes, commit keys, logical-key indexes, and derived projections.
The filesystem artifact store keeps immutable canonical bytes by digest for local development,
tests, replay fixtures, and typed workflow ports.

## Runtime

The runtime authority contract starts from `CertifiedTypedSpec`, not from parsed spec JSON or a
hash-only envelope. Before `RunStarted`, the assembly/runtime boundary verifies:

- spec hash and schema/version fields
- staged spec/certificate/config/seed artifact bytes and typed evidence
- descriptor identities and executable identity requirements
- runner registry availability
- capability registry availability

Runtime mutation middleware owns all execution appends. Bootstrap verifies and stages launch
material, executes the sealed `BootstrapRun` genesis state, and commits `RunStarted`, bootstrap
attempt lifecycle, launch artifact references, retention refs, and admitted artifact evidence in one
prepared store commit. Ordinary states, `PublicOutputRender`, `ProjectRetentionManifest`, and
`CompleteRun` use the same middleware path: staged artifacts are persisted before the prepared
commit, and run-store artifact evidence is admitted only in the commit that first references it.
Failed commits may leave orphan artifact-store bytes, but orphan run-store evidence is not
authority.

The scheduler materializes state inputs from certified binding trees and prior typed cell evidence.
It executes states in certified topological order and commits terminal evidence only through runtime
middleware and the prepared typed store boundary. Missing runners, missing capabilities, mismatched
specs, missing inputs, and malformed history fail before semantic execution advances.

Resume loads the stored certified spec, rebuilds projections from the run stream, verifies completed
cell and side-effect evidence against the spec, then advances only from a type-valid frontier.

Replay loads the stored certified spec and certificate artifacts, verifies them against the
production registry, compares the hashes to `RunStarted`, rebuilds stream evidence, and uses replay
adapters only. Live capability construction during replay is a contract violation.

## Side Effects

Side-effect states are multi-commit protocols. The scheduler/store own:

- logical ledger key derivation
- idempotency key stability
- claim generation and fencing
- invocation epoch
- durable transition ordering
- ambiguous recovery blocking
- terminal output binding

The durable uncertainty boundary is the invocation-started event. After that boundary, resume must
recover or block using typed evidence; it must not duplicate an external mutation or guess from
unstored state.

Prepared-invocation artifacts may retain unsigned mutation plans, expected hashes, and non-secret
signer references. Signed raw transactions are bearer mutation material and remain transient
submit-time bytes inside the mutation adapter.

Submission observed, submission unknown, and not-submitted-proven evidence share one logical
submission-result slot for an invocation epoch. Unknown submission can be superseded only by the
legal recovery transitions enforced by the store.

## Public Outputs

Public-output specs are part of the certified spec. Runtime public output is produced by typed
render states and events. Rendered JSON artifacts are cache material and must be checked against
typed output evidence before use.

Rendering helpers require `PublicOutputReadAuthority`, which `mfm-app` mints only after verifying
stored certified authority and rebuilding the projection from the authoritative run stream.
Rendered JSON cannot be used as resume, replay, certification, or render authority.

CLI and REST outputs are public API surfaces, but they are not semantic execution authority.

## CLI And REST Boundaries

`bin/cli` and `bin/rest-api` may:

- decode JSON/TOML/user input
- build typed configs and certified specs through operation crates
- select typed stores, artifacts, runners, and capabilities
- start, resume, replay, inspect, and render typed runs through app services
- preserve stable response envelopes

They must not:

- plan workflow semantics directly
- execute state behavior directly
- bypass typed certification
- infer public outputs from untyped snapshots
- migrate uncertified historical runs into certified typed runs

## Current Typed Workflow Ports

The active typed workflow ports are:

- proof: deterministic proof fact, side effect, output, and replay verifier contracts
- portfolio tracker: typed multi-network snapshot, fanout/fanin, publication, report, and public
  output
- EVM deploy/configure/validate: typed lifecycle side effects, protected transaction artifacts,
  validation reads, and replay verification

The current inventory is maintained in `docs/ops-and-states.md`.

## Documentation Update Rules

Update this document when a change alters:

- certified spec semantics
- typed event schemas
- store commit or projection authority
- resume or replay semantics
- effect/capability rules
- public-output authority
- crate ownership boundaries
- CLI or REST runtime contracts

Use `docs/architecture.md` for the short contributor map and `RFC_TYPED_CORE_PROPOSAL_1.md` for the
historical proposal that introduced this rewrite.

The former typed-core source-scan gates and summary-key CI scripts have been deleted. Real
guarantees now live in typed APIs, private constructors, crate dependency boundaries, Rust tests,
trybuild fixtures, cargo-metadata checks, and production-path integration tests.
