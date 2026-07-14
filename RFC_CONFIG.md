# RFC: configuration authoring current state, problem statement, and proposed solution 1

Status: problem definition with a preferred first proposal; design not yet accepted

Date: 2026-07-13

## Summary

MFM has strong configuration boundaries once a run is planned: semantic config is typed,
validated, canonicalized, content-addressed, certified, and kept free of secrets; runtime config is
process-local wiring and is excluded from replay authority. The current problem is above those
boundaries, in the operator authoring experience.

Today the public unit of configuration is one entry-point operation invocation. A real task often
requires several independently certified runs, so the user must author and keep several unrelated
TOML documents aligned by repeating network ids, chain identity, source identity, addresses, and
other shared intent. A dual-network portfolio snapshot currently needs three semantic config files
plus a runtime config file, followed by a manually ordered sequence of collector and report starts.
Each document can be valid on its own while the set is inconsistent.

The platform has no higher-level, typed authoring unit that describes the user's complete intent,
resolves reusable configuration, derives operation inputs, or validates their joins. The
entry-point registry can list names, versions, and accepted encodings, but it cannot describe
config schemas, produce examples, validate a file offline, or expose relationships between
operations.

This document records the current implementation and problem, then proposes a typed,
PostgreSQL-backed configuration catalog as the preferred first solution. The catalog is limited to
setup-time authoring and pre-planning resolution. Existing operation, state, capability, runtime,
certification, and replay responsibilities remain authoritative after resolution.

## Scope

This RFC covers configuration involved in starting public MFM operations:

- authored semantic config accepted by public entry-point operations;
- canonical typed config retained in certified specs;
- process-local runtime routing, signer, and keystore config;
- CLI and REST config transport behavior;
- the multi-run authoring experience, especially collect-then-report;
- config discovery, validation, versioning, provenance, and documentation.

It does not treat Cargo manifests, Nextest settings, Nix flake inputs, or SQLx metadata as product
configuration. Nixfied is relevant only where it generates or supplies MFM runtime configuration.

## Terms

### Authored config

User-supplied TOML or JSON for one public entry-point operation. It is semantic input and must not
contain secrets or process-local resource paths.

### Canonical typed config

Validated semantic config lowered into operation and state config material. It is canonicalized,
content-addressed, included in the certified typed execution spec, and admitted with the run.

### Runtime config

Process-local mapping from non-secret semantic references to concrete resources. It may resolve RPC
endpoints, authorization material, keystore paths, unlock files, and signer bindings. It is live
execution wiring, not run or replay authority.

### Process configuration

Binary and deployment settings such as `DATABASE_URL`, `MFM_REST_API_ADDR`, process role, and
output format. These settings are neither entry-point semantic config nor capability routing
config.

## Architectural constraints already in force

Any future design must retain the following properties:

- Operations remain deterministic planners from typed config to typed program graphs.
- State logic does not read files, environment variables, or live transports.
- Semantic config contains domain intent and non-secret references only.
- Runtime config remains process-local and must not enter specs, events, facts, artifacts, public
  output, or replay input.
- Hashed structured config contains no floating-point values.
- Certified specs remain the runtime contract.
- Collector and report operations remain independently reusable typed operations.
- `portfolio_snapshot` remains report-only. A distinct higher-level operation may compose typed
  collector and report operations into one certified graph when one workflow requires both.
- Cross-operation dependencies belong to operation composition and the resulting typed graph, not
  to the configuration catalog.
- Replay and evidence-only reads do not load runtime config.
- Secrets, secret-bearing paths, and signed mutation material remain below typed semantic surfaces.

The problem is therefore not that semantic and runtime configuration are separate. That separation
is intentional. The problem is that users must manually assemble and synchronize both sides, and
multiple operation inputs, without an aggregate authoring contract.

## Current configuration pipeline

```text
CLI file or REST config value
  -> mfm-authored-config parses declared TOML/JSON
  -> app entry-point registry resolves public op name + version
  -> EntryPointPlannerAdapter deserializes the op-specific Rust config type
  -> operation crate validates/normalizes and plans a typed program draft
  -> program lowering emits operation/state config material
  -> certifier validates and hashes the typed execution spec
  -> app admits content-addressed config artifacts with RunAdmitted
  -> runtime binds certified semantic refs to process-local runtime config when live IO is needed
```

### Authored ingress

`crates/authored-config` owns the shared ingress envelope:

- TOML and JSON are supported.
- Payloads are limited to 256 KiB by default.
- Invalid UTF-8, invalid syntax, duplicate JSON keys, floats, and unknown submitted fields are
  rejected.
- Parsed values are round-tripped through the selected Rust config type and canonical JSON.
- Original authored bytes and canonical normalized bytes are both digested.

There is an important provenance gap after normalization. `EntryPointPlannerAdapter::plan` passes
only the typed `normalized.value` to the operation planner. The authored-byte digest and normalized
canonical digest are not carried in `EntryPointOpPlan` or `EntryPointLaunchEvidence`. The certified
spec retains the semantic operation/state config material, but it does not retain the identity of
the source document as authored.

### Entry-point registry

`crates/app/src/entry_points.rs` manually registers each public op by connecting:

- an `EntryPointDescriptor`;
- a concrete Rust config type inferred through the planner function;
- a planner function;
- an app-level error mapper.

`EntryPointDescriptor` currently exposes only:

- namespace and internal name;
- public name;
- integer public version;
- accepted config formats.

It does not expose the authored config schema id, a machine-readable schema, a human description,
defaults, examples, runtime capability requirements, output schemas, related operations, or a
config migration contract. Consequently, `mfm ops list` is discovery of names, not discovery of how
to configure or operate them.

### Operation-owned config

Config ownership follows more than one pattern:

- Portfolio has a dedicated authored/canonical pipeline in `crates/portfolio-config`, then lowers
  into `PortfolioWorkflowConfig` in the portfolio state crate.
- BTC and EVM balance collectors use their operation config types directly from their op crates.
- EVM contract entry configs live in `crates/evm-contract-config` and compose context, action, import,
  transaction, receipt, assertion, and signer-intent types from the contract model/config crates.

All paths eventually become typed program config and are certified, but there is no uniform
authored-to-canonical lifecycle shared by every public op. The portfolio config crate also exposes a
path-hint parser that can infer TOML or JSON; the public CLI start path does not use it.

### Certification and persistence

Operation planning produces a `TypedProgramLaunchPlan` containing a draft plus config and seed
material. App assembly lowers and certifies the draft, then admits the required config artifacts in
the same run-start authority path. This correctly makes the effective semantic config replayable
and content-addressed.

The persisted config artifacts are the configs used by operation/state nodes, not a durable project
or workspace document that relates several entry-point runs. There is no persisted aggregate saying
that particular collector runs were authored as inputs for a particular portfolio intent.

## Public entry-point inventory

The production registry currently publishes seven version-1 operations. Every operation accepts
TOML and JSON.

| Public op | Authored Rust type and owner | Main semantic content | Live runtime config | Tracked complete TOML example |
|---|---|---|---|---|
| `portfolio_snapshot` | `PortfolioSnapshotAuthoredConfig` in `crates/portfolio-config` | Portfolio id, quote codes, networks, wallets, symbols, valuation config | None for chain access; reads admitted Platform facts through the store | `examples/configs/portfolio-dual-mainnet.toml` |
| `btc_address_balance` | `BtcAddressBalanceConfig` in `crates/ops/btc-collectors-op` | Network id, Bitcoin network tag, semantic source identity, addresses, coverage, read bound | BTC route keyed by semantic source identity | `examples/configs/btc-address-balance.toml` |
| `evm_native_balance` | `EvmNativeBalanceConfig` in `crates/ops/evm-collectors-op` | Network id, expected chain id, accounts, coverage, decimals, read bound | EVM route keyed by semantic network id | `examples/configs/evm-native-balance.toml` |
| `evm_contract_deploy` | `EvmContractDeployEntryConfig` in `crates/evm-contract-config` | Lifecycle context and deploy action | EVM route plus signer/keystore binding | None |
| `evm_contract_configure` | `EvmContractConfigureEntryConfig` in `crates/evm-contract-config` | Lifecycle context, deployed-stage import, configure action | EVM route plus signer/keystore binding | None |
| `evm_contract_validate` | `EvmContractValidateEntryConfig` in `crates/evm-contract-config` | Lifecycle context, configured-stage import, validation action | EVM route; no signer is semantically required | None |
| `evm_contract_lifecycle` | `EvmContractLifecycleEntryConfig` in `crates/evm-contract-config` | Lifecycle context and deploy/configure/validate actions | EVM route plus signer/keystore binding | None |

The internal BTC chain-head checkpoint operation is intentionally not a public entry point.

The tracked EVM contract documentation provides schematic JSON shapes, while the CLI example refers
to a local `lifecycle.toml` that is not present as a complete tracked example. Users must consult
domain documentation, Rust types, or test fixtures to construct these configs.

## Runtime config today

`crates/runtime-config` owns one TOML-or-JSON file with four top-level families:

| Family | Purpose |
|---|---|
| `evm` | RPC sources, ordered source policies, and routes from semantic network ids |
| `btc` | Bitcoin RPC routes keyed by semantic source identity |
| `keystores` | Process-local keystore and unlock-file profiles |
| `signers` | Non-secret signer refs mapped to concrete provider/profile entries |

Endpoint and credential fields support direct, environment, file, and environment-to-file
indirection. Runtime config parsing validates internal references and redacts values in diagnostics.
Expected EVM chain id and Bitcoin network tag remain semantic config; they are checked against the
live provider rather than copied into runtime source definitions.

CLI `run start` and `run resume` select the file with `--runtime-config`, falling back to
`MFM_RUNTIME_CONFIG_FILE`. REST live services use `MFM_RUNTIME_CONFIG_FILE`. Read-only CLI commands,
REST startup, status, stream inspection, replay, and public-output rendering do not load it.

App assembly builds one `LiveTransportRuntime` shared by all registered live runner families. The
file is loaded lazily and cached on first use. The current app loader calls the full
`RuntimeConfig::load_path`, so every present family is parsed together. Selective requirements exist
in `mfm-runtime-config`, but live app assembly does not use them. As a result, malformed signer data
can reject a validation-only EVM contract start even though that operation does not semantically
need a signer. The integration tests explicitly lock in that current behavior.

Nixfied adds another production path for runtime config creation: Reth-backed tasks generate a
runtime TOML file under the Nixfied state directory and export `MFM_RUNTIME_CONFIG_FILE`. This is a
safe deployment convenience, but it is separate from entry-point authored config and from the
tracked examples used by operators.

## CLI and REST behavior

### CLI

The start contract is:

```text
mfm run start --op <NAME> --config <PATH> [--op-version <N>]
              [--config-format <toml|json>] [--runtime-config <PATH>]
```

Current characteristics:

- `--op` and `--config` are both required.
- The config document does not identify its own op or version; that association exists only in the
  command line.
- Omitted `--op-version` selects the latest registered version.
- `--config-format` defaults to TOML without consulting the file extension. A JSON file requires an
  explicit flag even though shared authored-config code has path-format helpers.
- Authored config cannot be supplied through stdin or an inline value.
- CLI connects to the run store and loads its store scope before app planning validates the selected
  op config. Invalid config therefore cannot be checked through `run start` without a configured,
  reachable store.
- `ops list` works offline, but reports only name, version, and accepted formats.
- There is no `ops describe`, config schema export, template generation, aggregate validation, or
  dry-run planning command.

### REST

REST accepts `op`, optional `op_version`, optional `config_format`, `config`, and optional
`invocation_key` in one JSON request.

- A structured JSON object or array implies JSON when `config_format` is omitted.
- A string config defaults to TOML when `config_format` is omitted.
- TOML must be embedded as a JSON string; JSON may be structured.
- REST obtains live services and the store scope before preparing and validating the entry-point
  launch, so it also lacks a store-independent validation surface.
- Runtime config is server process state rather than request data, as required by the security and
  replay model.

The CLI and REST ultimately share `AuthoredConfig` and app planning, but their format detection and
document transport behavior differ.

## The representative failure: collect then report

The dual-mainnet portfolio recipe is the clearest expression of the problem. It requires:

1. a BTC collector config;
2. an EVM collector config;
3. a portfolio report config;
4. a runtime routing config;
5. three separately issued start commands in the correct order.

The repeated data is not incidental:

| Concept | BTC collector | EVM collector | Portfolio report | Runtime config |
|---|---:|---:|---:|---:|
| Semantic network id | yes | yes | yes | EVM route key |
| Expected EVM chain id | no | yes | yes | deliberately no |
| Bitcoin network tag | yes | no | yes | deliberately no |
| Bitcoin semantic source identity | yes | no | yes | BTC route key |
| Wallet/account address | addresses | accounts | wallet subjects | no |
| Native asset decimals | no | yes | no; consumed from selected fact evidence | no |
| Coverage/read policy | yes | yes | selection expects acceptable coverage | no |

The architecture correctly keeps the collector and report runs separate. However, the operator is
acting as the compiler between one portfolio intent and three per-op configs. The repository's
operator recipe literally describes the collector files as separate tracked inputs and instructs
the user to rerun collectors before the report.

The local repository convention reinforces the scattered workflow: root-level `*.toml` files are
ignored, while tracked examples live under `examples/configs`. A normal operator working set can
therefore accumulate independent portfolio, collector, and runtime files at the repository root
without a checked manifest that explains their relationship. The present working copy exhibits
exactly that shape.

## Problem statement

### 1. The configuration unit does not match the user intent unit

The platform asks for one config per operation invocation. Users think in terms of a portfolio,
contract lifecycle, deployment, or recurring operational objective. When one objective spans
multiple certified runs, MFM exposes the runtime decomposition directly as the authoring model.

### 2. Shared semantic facts have multiple writable copies

Network ids, chain ids, network tags, source identities, addresses, and asset details are repeated
across configs. There is no declared source of truth and no deterministic derivation contract. A
change must be copied manually to every affected file.

### 3. Validation is local, not relational

Each config type validates its own shape and invariants. No current command validates that:

- collector subjects are exactly those required by a portfolio;
- collector network identity agrees with the portfolio network declaration;
- every semantic runtime ref has a matching process route;
- every mutation signer ref has a matching runtime signer profile;
- all files target compatible entry-point versions;
- a multi-run sequence is complete before any run starts.

Many mismatches therefore appear as launch ingress errors or later `missing_fact` /
`no_common_network_anchor` report failures rather than authoring-time diagnostics.

### 4. Entry-point discovery stops before configuration discovery

The registry knows how to deserialize each concrete config type through Rust generics, but that
knowledge is not represented in public entry-point metadata. Users cannot ask the binary for the
selected op's schema, fields, defaults, examples, runtime dependencies, or related operations.

### 5. Authored documents are weakly identified

A standalone config file does not declare which op or version consumes it. The caller supplies both
out-of-band, and omitting the version means "latest." File names and directory placement are only
conventions. The original authored document digest is calculated but is not retained as entry-point
launch evidence.

### 6. Parsing and validation behavior is inconsistent across surfaces

The shared config crate has path-based format detection, but CLI start defaults unconditionally to
TOML. REST infers JSON only for structured JSON values. Portfolio has an explicit authored and
canonical pipeline; collector and contract configs take different paths to the same typed-program
boundary. Error mapping also differs by op and often collapses detailed typed failures into generic
decode or plan errors.

### 7. Runtime configuration is correctly separate but operationally entangled

Semantic refs must match runtime route/profile keys through manually repeated strings. One shared
runtime file is parsed as a whole when first needed, so malformed config in an unrelated present
family can block an operation. Users also have to understand when runtime config is required,
optional, ignored, or deliberately forbidden, but the entry-point registry does not expose that
information.

### 8. Examples and docs are the de facto schema catalog

The tracked collect-then-report examples are tested and useful, but only three of seven public ops
have complete authored TOML examples. Runtime schemas are documented in separate EVM and Bitcoin
runbooks. Contract lifecycle shapes are spread across domain documentation, Rust types, and test
fixtures. The CLI documentation also lists `run start` flags for framework version and source
revision that are not present in the current `StartArgs`. Documentation drift is already an API
discovery failure, not only a future risk.

### 9. There is no config lifecycle beyond deserialize-or-fail

Public ops have versions, typed config schemas have ids, and persisted config is content-addressed,
but authored files have no common envelope, declared schema version, compatibility range, migration
path, deprecation metadata, or formatter. A future version can be selected by "latest" while the
file itself carries no statement of intent.

## Concrete failure scenarios

### Stale collector subject

An address changes in the portfolio config but not in its collector config. The collector succeeds
for the old address. The report later fails because the required fact for the new subject does not
exist. Both files were individually valid.

### Network identity drift

An EVM collector and portfolio use different chain ids for the same textual network id, or a BTC
collector and portfolio use different network tags/source identities. There is no aggregate
authoring check; the failure occurs in separate planning, provider-binding, or fact-selection paths.

### Missing runtime join

Semantic config names a network or signer ref that has no matching runtime route/profile. The
semantic file is valid and certifiable, but live launch cannot bind the required capability.

### Unrelated malformed runtime family

A validation-only EVM contract op needs an EVM read provider but no signer. If the shared runtime
file contains malformed signer config, full-file parsing rejects the start before admission.

### Format surprise

A user passes a `.json` file to CLI start and omits `--config-format json`. The CLI labels the bytes
as TOML instead of deriving the format from the path, producing a syntax failure unrelated to the
selected op's actual schema.

### Version ambiguity

A valid standalone TOML file is launched without `--op-version`. Its intended schema version is not
recorded in the file; resolution uses whichever version is latest in the running binary.

## Consequences

- Routine operation requires copy/paste and shell orchestration.
- Small changes create multi-file drift risk.
- Correctness is discovered too late, often after live dependencies and storage are involved.
- New entry points increase documentation and example burden linearly.
- Automation must hard-code knowledge that the compiled registry already has implicitly.
- Operators can confuse semantic config, runtime config, and process config because all are commonly
  represented as TOML and selected through adjacent command-line options.
- AI and programmatic clients receive stable run output envelopes but lack an equally stable
  machine-readable input contract.
- The current model is difficult to extend to recurring runs, fleets of portfolios, multiple
  environments, or reusable organization-wide network/signer profiles without multiplying files.

## What is not broken

The following should not be "fixed" by weakening existing boundaries:

- Per-op typed configs are appropriate certified planning inputs.
- Collector and report operations must preserve their independent typed contracts, and
  `portfolio_snapshot` must remain report-only.
- A higher-level operation may reuse collector and report operation builders. Their ordering and
  data dependencies then belong to that operation's certified typed graph, while the report still
  consumes admitted facts rather than live chain providers.
- Runtime endpoints and secret-bearing paths must not move into semantic config for convenience.
- Replay must not resolve current runtime profiles.
- Content-addressed canonical config inside the certified spec is the right execution authority.
- Strict unknown-field, no-float, validation, and redaction behavior should remain fail-closed.

A long-term design should add a coherent authoring and compilation layer around these contracts,
not replace them with an untyped universal bag of settings.

## Requirements for a future solution discussion

This section defines outcomes, not an implementation.

1. A user can express one operational intent without hand-maintaining duplicate shared fields.
2. The system can deterministically produce every required typed operation config. A higher-level
   operation, rather than the config layer, composes multiple operations when the intent requires a
   larger graph.
3. Catalog resolution may read PostgreSQL, but typed resource validation and config construction do
   not use live RPC, keystores, secrets, environment variables, clocks, or other ambient input.
   Resolved immutable revisions can be exported and revalidated without the catalog database.
4. Existing typed capability contracts and runtime binding checks remain responsible for runtime
   completeness; the catalog does not duplicate capability declarations or persist live resources
   into certified semantics.
5. Public entry-point metadata is sufficient for machine discovery, validation, examples, and
   version-aware tooling.
6. The effective config for every certified run remains explicit, canonical, content-addressed, and
   replayable.
7. Authored source provenance and generated config provenance have an intentional contract.
8. CLI and REST share equivalent config semantics even if their transport encodings differ.
9. Partial config, stale fields, ambiguous precedence, and accidental secret promotion fail closed.
10. Existing op/state/adapter/transport boundaries and the report-only, fact-backed authority split
    are preserved, including when a higher-level operation composes collector and report ops.

## Proposed solution 1: typed configuration catalog

### Decision summary

Introduce a general configuration module backed by PostgreSQL. It stores reusable, versioned typed
configuration resources created during a separate setup or onboarding phase. Before operation
planning, an application-layer resolver loads the exact catalog resources referenced by an
entry-point request and gives their typed values to an operation-owned config builder. The builder
deterministically produces the operation's complete `MfmConfig`.

The catalog has no responsibility after that point:

```text
TOML import, CLI, REST, or onboarding UI
  -> validate and publish typed catalog revisions

entry-point request with typed catalog refs
  -> application-layer catalog resolver performs PostgreSQL reads
  -> operation-owned pure config builder produces complete MfmConfig
  -> Operation::expand composes states and nested operations
  -> certify and admit the typed graph
  -> existing state/adapter/transport runtime
```

The catalog is an authoring and resolution facility, not runtime authority, workflow orchestration,
or a replacement for typed operation config.

### Responsibility boundary

The proposed configuration module owns:

- generic persistence for typed catalog resources and immutable revisions;
- exact-reference and human-facing alias resolution;
- schema identity checks and decoding through registered domain-owned resource types;
- transactional publication of a coherent set of validated revisions;
- source revision and digest provenance for config construction;
- import and export surfaces for setup TOML, CLI, REST, and future onboarding clients.

The module does not own:

- operation or state graph topology;
- dependencies between states or nested operations;
- declarations of state runtime capabilities;
- capability-provider selection or availability checks;
- transport, signer, keystore, or secret resolution;
- operation semantic validation after a complete `MfmConfig` exists;
- runtime execution, resume, replay, or fact-selection authority.

PostgreSQL access must remain outside `Operation::expand`. Operations stay deterministic and do not
receive a database handle, repository trait, or generic key/value lookup API. The application layer
resolves typed references first, and the operation receives only typed resolved values.

### Typed resources, not a universal settings object

The catalog storage mechanism is general, but the values are not an untyped bag of JSON settings.
Domain crates own concrete resource types, schema identities, decoding, and validation. Initial
resource families are expected to include:

- EVM and Bitcoin semantic network definitions;
- EVM account sets and Bitcoin address sets;
- token and asset definitions and named token sets;
- portfolio, organization, and wallet subject groupings;
- non-secret signer intent and expected public signer identity.

Each reference carries the expected resource type at the Rust boundary. Conceptually:

```rust
struct EvmBalanceRequest {
    network: CatalogRef<EvmNetwork>,
    accounts: CatalogRef<EvmAccountSet>,
    policy: BalancePolicy,
}

struct EvmBalanceResolved {
    network: Resolved<EvmNetwork>,
    accounts: Resolved<EvmAccountSet>,
    policy: BalancePolicy,
}
```

The operation-owned config builder accepts `EvmBalanceResolved` and produces the existing complete
`EvmNativeBalanceConfig`. The catalog cannot manufacture operation config through field names,
string interpolation, dynamic maps, or generic merge rules.

This proposal explicitly rejects:

- arbitrary `get("some.path")` access from operations or states;
- textual TOML includes;
- generic deep merge or environment overlay precedence;
- mutable catalog values read during execution;
- silent schema migration during config resolution;
- resolving durable launches against an implicit `latest` revision.

### Resource identity and persistence

A minimal relational model separates a stable human-facing resource from its immutable content:

```text
catalog_entry
  id
  namespace
  name
  kind

catalog_revision
  id
  entry_id
  revision
  schema_id
  canonical_payload
  canonical_digest
  created_at
```

An optional publication or catalog-release record may group several revisions that onboarding must
publish atomically. A mutable alias may point to the preferred revision for interactive authoring,
but operation config resolution records and uses the exact revision id, schema id, and canonical
digest. Published revision payloads are append-only and retained for provenance.

PostgreSQL is the initial persistence implementation, not the semantic identity of the catalog.
Typed resolution produces a closed set of immutable values that can be exported, hashed, tested,
and revalidated without subsequent database access.

### Config construction contract

Each catalog-backed entry point has two distinct inputs:

1. A small authored request containing typed catalog references and operation-local policy.
2. The complete canonical `MfmConfig` produced after resolution.

The application-layer flow is:

1. Parse the entry-point request and determine its expected typed references.
2. Resolve every reference to an exact immutable revision in one coherent snapshot.
3. Verify resource kind, schema identity, digest, reference closure, and configured size limits.
4. Decode each revision into its domain-owned Rust type and run resource validation.
5. Pass only typed resolved values to the operation-owned pure config builder.
6. Validate the resulting `MfmConfig` through its existing operation contract.
7. Expand, lower, certify, and admit through the existing path.

The builder is deterministic over its request and resolved values. It performs relational semantic
checks that an individual catalog resource cannot perform, such as ensuring that a token set and an
account set belong to the selected EVM network. The operation's existing config validation remains
the final planning check.

The simplest implementation may define this as an entry-point config-builder contract rather than
changing the kernel `Operation` trait. Catalog resolution is pre-planning application behavior;
`Operation::expand` continues to accept only its complete typed config.

### Operation composition owns dependencies

The catalog does not describe operational gates, run ordering, or state dependencies. MFM already
supports an operation calling registered typed operation builders. When a workflow requires several
operations, a developer defines a higher-level operation that composes them.

For collect-then-report, a distinct higher-level operation may:

1. reuse the BTC collector operation;
2. reuse the EVM collector operation;
3. connect their typed completion and fact-production path to the report operation;
4. expose the final report output.

The resulting graph, operation lineage, state dependencies, fact writes, and report reads are
planned and certified by the existing typed program machinery. `portfolio_snapshot` itself remains
report-only and does not gain live chain capabilities. The individual collector and report
operations also remain usable independently.

If a deployment deliberately launches those operations as separate runs instead, ordering remains
an external workflow concern. It is still not catalog behavior.

### Existing capability typing remains authoritative

The proposed catalog does not introduce a parallel `required_runtime_capabilities` declaration.
States already express typed capability contracts, adapters bind those contracts, and live ingress
rejects missing or invalid runtime providers. Config construction supplies semantic references such
as network or signer identity; it does not choose concrete RPC clients, keystores, or transports.

Tooling may later derive a non-authoritative capability preview from operation metadata or a planned
spec, but the catalog neither stores nor enforces a duplicate capability summary.

### Semantic catalog and runtime setup

One onboarding experience may configure both semantic resources and runtime wiring, but those are
different typed surfaces with different persistence and security rules.

The semantic catalog may contain:

- network identity, expected chain id, and Bitcoin network tag;
- token contract identity, symbol, and integer/decimal-string metadata;
- public account and address sets;
- semantic source identity;
- non-secret signer refs and expected public signer addresses.

The semantic catalog and generated `MfmConfig` must not contain:

- RPC credentials or credential-bearing URLs;
- mnemonics, private keys, passwords, or unlock material;
- keystore or secret-bearing file paths;
- concrete signer providers or live transport instances.

RPC route and signer-provider onboarding feeds the existing runtime configuration and capability
binding layer. Whether runtime profiles eventually gain their own database-backed management
surface is a separate security-sensitive design decision; it is not part of this proposal.

### Setup and onboarding

The same typed publication API supports an initial TOML import and future interactive onboarding:

```text
mfm setup import organization.toml
  -> parse typed resource documents
  -> validate each resource and all declared references
  -> show the proposed revision set
  -> publish atomically
```

CLI, REST, and a UI can then add or revise networks, public wallet identities, token sets,
portfolio groupings, and non-secret signer intents without creating one handwritten TOML file per
operation. TOML and JSON become import/export encodings for typed catalog resources rather than the
long-term source of operational composition.

Setup validation has three layers:

1. Resource validation at publication, such as address shape and chain-id policy.
2. Relational validation during operation config construction, such as network agreement between
   selected resources.
3. Existing `MfmConfig` validation and deterministic operation expansion over the complete config.

### Certification, provenance, and failure behavior

The generated complete `MfmConfig` and certified spec remain execution and replay authority. The
catalog is not consulted by runtime, resume, replay, public-output rendering, or evidence-only
reads.

Launch provenance should retain, directly or through one content-addressed receipt:

- the entry-point request digest;
- exact catalog revision ids, schema ids, and canonical digests;
- the config-builder identity/version;
- the generated canonical operation-config digest;
- the resulting certified-spec identity.

This provenance explains how the config was constructed but does not authorize execution
independently of the certified spec.

The failure boundary is intentional:

- catalog unavailability can block preparation of a new run;
- it cannot affect an admitted run, resume, or replay;
- changing an alias affects only future resolutions;
- changing semantic input publishes a new immutable revision;
- config construction fails closed on missing, incompatible, cyclic, oversized, or ambiguous
  references;
- no successful resolution can leave the operation with a partially populated config.

### Expected UX improvement

The user authors reusable domain information once during setup. Entry-point requests select those
resources and add only operation-local policy. Developers retain all workflow semantics in typed
operations and all execution semantics in typed states.

The intended division is:

```text
catalog:    what reusable typed semantic values are available?
builder:    how does this operation construct its complete config from them?
operation:  what typed graph implements the requested workflow?
state:      what reusable domain transition occurs at runtime?
adapter:    how is the state's capability contract bound?
transport:  how does live protocol IO happen?
```

## Questions to resolve next

- Which crate owns the catalog traits and generic resolver without making kernel crates depend on a
  storage implementation?
- How do domain crates register catalog resource codecs, schema descriptors, and validators without
  recreating manual app wiring for every resource type?
- What is the exact typed request/config-builder API, and how does it coexist with direct complete
  per-op config authoring during migration?
- Does publication need first-class catalog releases, or are immutable revision ids plus a
  transactionally resolved lock sufficient?
- What are the compatibility and explicit migration contracts for resource schemas, authored
  requests, config builders, and generated operation configs?
- Which provenance fields belong in run admission evidence versus a separate content-addressed
  config-construction receipt?
- How are catalog authorization, tenant/namespace isolation, retention, and audit enforced?
- Which non-secret runtime-profile declarations, if any, may be managed by the onboarding surface
  while live bindings and secrets remain outside semantic config?
- What import/export envelope permits exact offline revalidation of a resolved revision closure?
- Which higher-level operations should be introduced first to replace the current external
  collect-then-report recipe?

## Evidence map

The current-state findings above are grounded in these repository surfaces:

- `crates/authored-config/src/lib.rs`: shared TOML/JSON ingress and normalization;
- `crates/app/src/entry_point.rs`: entry-point descriptor, adapter, and registry;
- `crates/app/src/entry_points.rs`: production registration of seven public ops;
- `crates/app/src/lib.rs`: entry-point planning, certification, and launch material;
- `crates/runtime-config/src/lib.rs`: runtime config schema, indirection, and validation;
- `crates/app/src/live_transports.rs`: lazy shared runtime-config loading and provider assembly;
- `crates/kernel/program/README.md` and `crates/kernel/program/src/lib.rs`: typed nested-operation
  composition through framework-minted `OperationExpansion`;
- `docs/architecture.md`: operation/state/adapter/transport placement and dependency boundaries;
- `bin/cli/src/commands/run/start.rs`: CLI file and format behavior;
- `bin/cli/src/commands/ops.rs`: current public discovery surface;
- `bin/rest-api/src/lib.rs`: REST config envelope and launch flow;
- `crates/portfolio-config` and `crates/portfolio/model`: portfolio authored/canonical config;
- `crates/ops/btc-collectors-op`: BTC collector config;
- `crates/ops/evm-collectors-op`: EVM collector config;
- `crates/evm-contract-config`: EVM contract entry configs;
- `docs/portfolio-collect-then-report.md`: current multi-run operator recipe;
- `docs/evm-rpc-routing.md` and `docs/btc-rpc-routing.md`: runtime routing contracts;
- `examples/configs`: the tracked authored/runtime examples;
- `.gitignore` and `nixfied.nix`: local root-TOML convention and generated runtime config path.
