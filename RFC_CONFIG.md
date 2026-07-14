# RFC: configuration authoring current state, problem statement, and proposed solution 1

Status: preferred design accepted for implementation planning

Date: 2026-07-14

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
certification, and replay responsibilities remain authoritative after resolution. The proposal is
a breaking replacement for direct per-operation authored config, not an additional path beside it.

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
5. Public entry-point metadata identifies each exact entry point and its request schema. Typed
   request decoding remains the validation authority; generic schema generation and example
   synthesis are not prerequisites for the first implementation.
6. The effective config for every certified run remains explicit, canonical, content-addressed, and
   replayable.
7. Exact catalog source identities and generated config provenance have an intentional contract.
8. CLI and REST share equivalent config semantics even if their transport encodings differ.
9. Partial config, stale fields, ambiguous precedence, and accidental secret promotion fail closed.
10. Existing op/state/adapter/transport boundaries and the report-only, fact-backed authority split
    are preserved, including when a higher-level operation composes collector and report ops.

## Proposed solution 1: typed configuration catalog

### Decision summary

Introduce a small configuration module backed by one append-only PostgreSQL table. It stores
reusable typed canonical values created during setup or onboarding. The content digest is the value's
revision identity; the initial design has no separate entry, revision, alias, release, dependency,
or lock model.

Before operation planning, an application-layer resolver loads the exact typed values referenced by
an entry-point request. An ordinary operation-owned pure function then produces the operation's
complete `MfmConfig`.

The catalog has no responsibility after that point:

```text
setup TOML import
  -> validate, canonicalize, and append typed catalog values

entry-point request with exact CatalogRef<T> values
  -> application-layer typed loader performs PostgreSQL reads
  -> ordinary operation-owned build_config function produces complete MfmConfig
  -> Operation::expand composes states and nested operations
  -> certify and admit the typed graph
  -> existing state/adapter/transport runtime
```

The catalog is an authoring and resolution facility, not runtime authority, workflow orchestration,
or a replacement for typed operation config.

### Minimal persistence model

A single table in the existing PostgreSQL store authority is sufficient:

```text
catalog_values
  name
  schema_id
  digest
  canonical_json

primary key
  (name, schema_id, digest)
```

`name` is a stable human-facing path such as `networks/ethereum-mainnet` or
`wallets/treasury-evm`. `schema_id` identifies the expected Rust config schema. `digest` identifies
the canonical value and serves as its revision. Canonical JSON bytes, rather than PostgreSQL JSONB
normalization, are the authoritative payload.

The table enforces conservative name, schema-id, digest, and 256 KiB payload bounds. The existing
append-only database trigger contract rejects update, delete, and truncate. An insert that conflicts
on the exact primary key is accepted only after the stored bytes are verified equal, so retries are
idempotent but hash/key collisions fail closed. Listing returns identity metadata only with bounded
keyset pagination; exact export returns canonical bytes after digest verification.

Publishing a changed value inserts another row under the same name with a new digest. The database
schema and existing store-scope authority provide isolation, so catalog identity does not duplicate
`scope_id`. Creation time is not semantic identity and is unnecessary while there is no "latest"
selector. The application role does not update, delete, or truncate published rows. A setup import
may insert several rows in one ordinary PostgreSQL transaction, but there is no first-class catalog
release or snapshot table.

The existing PostgreSQL storage crate, pool, migrator, and schema authority are reused. Because the
crate will no longer store only streams, the implementation should rename it and its concrete store
type to general PostgreSQL storage names and delete the old names. It must not introduce a second
PostgreSQL crate, pool, migration authority, app-level storage alias, or compatibility alias.

PostgreSQL is the first persistence implementation, not part of catalog value identity. An exact
reference and its canonical bytes are sufficient to verify a value without later database access.

### Reuse the existing typed config contract

The catalog storage mechanism is general, but the values are not an untyped bag of JSON settings.
Domain crates own concrete Rust types, schema descriptors, decoding, and validation. The design
reuses `MfmConfig` directly for catalog values because it already describes deterministic config
safe to persist, exposes a schema id, has a semantic validation hook, and prohibits secrets. There
is no `CatalogValue` marker trait or second schema and validation framework. Setup import supports a
closed list of concrete resource document types; it is not a public generic "persist any JSON"
surface.

The first implementation stores existing complete `MfmConfig` types rather than inventing granular
resource wrappers before they have multiple consumers. Its value set is `PortfolioConfig`, the BTC
and EVM collector configs, `EvmContractContext` promoted to `MfmConfig`, and the existing contract
action/import config types. The composed portfolio operation derives collector configs from one
loaded `PortfolioConfig`, so the motivating workflow still has one source for networks, wallets,
and symbols. Separate network, account-set, address-set, token-set, and signer-intent catalog types
are deferred until a concrete second consumer justifies them.

The only new typed primitive required by the core model is an exact reference:

```rust
struct CatalogRef<T: MfmConfig> {
    name: ConfigName,
    digest: ContentDigest,
    _type: PhantomData<fn() -> T>,
}
```

The generic PostgreSQL loader conceptually exposes:

```rust
async fn load<T: MfmConfig>(reference: &CatalogRef<T>) -> Result<T>;
```

`load` verifies the exact row, expected `T::schema_id()`, canonical bytes, content digest, strict
decode, and `T::validate()` before returning `T`. There is no public generic
`get("some.path") -> JSON` interface and no catalog handle reaches an operation or state.

An entry-point request uses typed references and operation-local policy:

```rust
struct EvmBalanceRequest {
    config: CatalogRef<EvmNativeBalanceConfig>,
}
```

No `Resolved<T>` framework type is required. The application loads ordinary typed values, and the
operation-owned builder accepts those values directly.

### Names are labels; digests are durable identity

Every operation request contains exact references. There is no name-only, latest, current, alias,
or implicit resolution path. Setup import returns the exact `(name, schema_id, digest)` identity,
and catalog listing exposes those identities so a CLI, REST client, or UI can select one explicitly.
Preview, approval, storage, delayed launch, and execution therefore use the same immutable value.

This intentionally gives up the convenience of "whatever is newest". If a future recurring-fleet
feature needs mutable environment pointers, that feature requires a separate design and must still
pin exact references before operation preparation.

### Versioning and explicit migration

Configuration versioning has four independent dimensions:

| Dimension | Durable identity |
|---|---|
| Catalog value content | Canonical content digest |
| Catalog value schema | `MfmConfig` schema id |
| Request and builder semantics | Public entry-point operation version |
| PostgreSQL representation | Ordinary database schema migration |

Changing only a value, such as adding an account to an account set, keeps the same typed schema but
produces new canonical bytes and therefore a new digest. Both rows remain available:

```text
wallets/treasury-evm
  schema EvmAccountSetV1, digest sha256:A
  schema EvmAccountSetV1, digest sha256:B
```

The digest is the authoritative content revision. A future numeric revision may improve display or
sorting, but it must not replace the digest in a durable reference.

A breaking structural or semantic change introduces a new Rust type and schema identity:

```text
networks/ethereum-mainnet
  schema EvmNetworkV1, digest sha256:C
  schema EvmNetworkV2, digest sha256:D
```

`CatalogRef<EvmNetworkV1>` cannot load the `EvmNetworkV2` row because the generic loader verifies
the exact schema id before decoding. A typed request carries `name` and `digest`; its exact request
schema supplies `T` and therefore the expected schema id. Catalog listing, exact export, and launch
provenance retain `name`, `schema_id`, and `digest` for language-neutral inspection.

Schema migration is an explicit pure transformation from one exact typed value to another:

```text
CatalogRef<EvmNetworkV1> @ sha256:C
  -> migrate_v1_to_v2
  -> validate EvmNetworkV2
  -> append CatalogRef<EvmNetworkV2> @ sha256:D
```

A migration never updates or deletes the source row, never runs implicitly inside `load`, and never
runs during operation launch. Setup tooling may expose migration commands, but the result is always
a reviewable newly appended value with a new exact reference.

The exact public entry-point id includes the operation version and declares the request and catalog
schemas its builder accepts. A new operation version may require a new catalog schema or may
explicitly support more than one version through typed variants. Similar field shapes never imply
compatibility, and the loader never guesses or silently upgrades a value. Public run ingress never
selects a latest operation version.

Changing PostgreSQL columns, indexes, or storage encodings is an independent database migration. It
must preserve the stored canonical bytes, schema ids, and digests and does not create a new semantic
catalog value by itself.

### Config construction stays operation-owned

Each catalog-backed entry point distinguishes:

1. A small authored request containing typed catalog references and operation-local policy.
2. The complete canonical `MfmConfig` produced after resolution.

When a request references one already-complete `MfmConfig`, exact loading is the entire construction
step; the implementation must not add an identity builder. When multiple values or operation-local
policy must be joined, the operation owns an ordinary pure function rather than implementing a new
universal builder trait:

```rust
fn build_config(
    request: EvmBalanceRequest,
    network: EvmNetwork,
    accounts: EvmAccountSet,
) -> Result<EvmNativeBalanceConfig>;
```

The application flow is:

1. Parse an entry-point request that already contains exact digests.
2. Load each `CatalogRef<T>` through the generic typed loader.
3. Use a loaded complete config directly, or pass multiple typed values to the operation-owned
   `build_config` function when assembly is required.
4. Validate the resulting complete `MfmConfig` through its existing operation contract.
5. Expand, lower, certify, and admit through the existing path.

The builder is deterministic over its request and resolved values. It performs relational semantic
checks that an individual catalog resource cannot perform, such as ensuring contract imports match
the selected context or deriving collector configs consistently from one portfolio. The operation's
existing config validation remains the final planning check.

No kernel trait changes are required. Catalog resolution is pre-planning application behavior;
`Operation::expand` continues to accept only complete typed config. The entry-point operation
version owns the semantics of request resolution and config construction; an incompatible builder
change requires a new public operation version rather than a separate builder-version system.

### Keep catalog values flat initially

The initial catalog does not support catalog values that import or reference other catalog values.
An account set, token set, or wallet grouping is a self-contained typed value. An operation request
selects every reusable value needed by its builder.

This deliberately avoids recursive dependency closure, cycles, import depth, lockfiles, and partial
resolution. Cross-value relationships are validated by `build_config` and by the final
`MfmConfig::validate()`. Nested catalog references may be reconsidered only if concrete duplication
cannot be modeled acceptably with self-contained sets and direct typed refs.

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

The same typed append API supports an initial TOML import and future interactive onboarding:

```text
mfm setup import organization.toml
  -> parse typed resource documents
  -> validate and canonicalize each value
  -> show names, schemas, and resulting digests
  -> append all rows in one transaction
```

The initial CLI imports complete typed values, including a `PortfolioConfig` aggregate that owns its
networks, public wallet identities, and symbols without one handwritten TOML file per collector and
report operation. TOML is the setup import encoding and canonical JSON is the exact export encoding.
Future REST or UI onboarding may edit those concepts separately and publish the resulting complete
typed value through the same boundary after its authorization model is designed.

Setup validation has three layers:

1. Resource validation at publication, such as address shape and chain-id policy.
2. Relational validation during operation config construction, such as network agreement between
   selected resources.
3. Existing `MfmConfig` validation and deterministic operation expansion over the complete config.

### Certification, provenance, and failure behavior

The generated complete `MfmConfig` and certified spec remain execution and replay authority. The
catalog is not consulted by runtime, resume, replay, public-output rendering, or evidence-only
reads.

No separate derivation receipt is required initially. Existing launch evidence should retain the
small source set:

- catalog name;
- exact schema id and digest selected for each value.

The generated canonical operation-config digest and certified-spec identity already exist in the
normal planning/admission path. Together these values explain construction without creating another
authority object.

The exact entry-point id and catalog sources are `RunAdmitted` audit evidence. They do not enter
`RunIdentityMaterialV1`; run identity remains defined only by certified spec hash, store scope, and
invocation-key digest as required by `docs/design.md`.

The failure boundary is intentional:

- catalog unavailability can block preparation of a new run;
- it cannot affect an admitted run, resume, or replay;
- publishing under an existing name appends a new digest and cannot mutate an exact ref;
- config construction fails closed on missing, incompatible, oversized, or ambiguous references;
- no successful resolution can leave the operation with a partially populated config.

### Required breaking cutover and deletion

The catalog replaces the current direct-authoring path in one cutover. The repository must not
retain both models, add fallback decoding, preserve old public names through aliases, or add legacy
event/schema readers. Once the catalog-backed start path is usable, the only semantic start path is:

```text
exact entry-point id + JSON typed-reference request
  -> load exact typed catalog values
  -> operation-owned build_config
  -> complete MfmConfig
  -> existing planning, certification, admission, and runtime
```

TOML remains an import encoding for setup and an encoding for process-local runtime configuration.
It is no longer a per-operation run-start encoding.

The implementation is expected to make the following hard deletions.

#### Remove duplicate config crates and canonicalization

- Delete `crates/authored-config` and its `AuthoredConfig`, `AuthoredConfigFormat`,
  `NormalizedAuthoredConfig<T>`, `EntryPointDescriptor`, format detection/fallback, source-digest,
  and TOML-to-JSON normalization paths.
- Delete `crates/portfolio-config` and its authored, canonical, build-report, parse, and
  canonicalization types.
- Use `mfm_portfolio_model::PortfolioConfig` directly as the portfolio tracker operation config.
  Delete `PortfolioWorkflowConfig` and its authored/canonical conversions.
- Retain one canonical config authority: `MfmConfig::validate`, `ValidatedConfig<T>`, canonical
  config binding, certified config artifacts, and typed-program config material. Domain
  normalization such as stable portfolio collection ordering remains because it defines semantics.
- Remove the hidden sample-network `Default` implementations from public BTC and EVM collector
  configs. Production builders and tests must provide explicit values.

This removes two crates and three representations of portfolio semantic config without replacing
them with equivalent catalog wrappers.

#### Collapse entry-point selection and preparation

- Replace the public name plus optional version contract with one exact entry-point id containing
  its version, for example `mfm.portfolio/portfolio_snapshot@1`.
- Delete latest-version resolution, accepted-format metadata, nested name/version maps, the global
  registry digest, public registry construction, and format-bearing descriptor constants in
  operation crates.
- Delete `LaunchableOp`, `EntryPointPlannerAdapter`, and public adapter/registration variants that
  exist only to feed the old authored-config path.
- Delete the app-level `EntryPointOpId`, `EntryPointOpError`, `EntryPointRunLaunchInput`,
  `PreparedEntryPointRunLaunch`, duplicate `EntryPointLaunchEvidence`,
  `production_entry_point_op_registry`, and registration helpers.
- Delete `ProductionRunStore` and `ProductionPostgresSchema` aliases rather than renaming them to
  another app alias. Use the concrete `PostgresStore` name or a narrow app service constructor.
- Keep a private application dispatch table from exact entry-point id to one typed request
  preparation function. Do not introduce a public dynamic builder trait or service-locator API.
- Preserve operation-owned `build_config` functions and ordinary operation-specific errors. Collapse
  app error wrappers that merely translate another error without adding a stable boundary.

An incompatible request or builder change creates another exact entry-point id. There is no public
`latest`, compatibility range, or implicit selection rule.

#### Replace CLI and REST run ingress

- Replace CLI `--op`, `--op-version`, `--config`, and `--config-format` with `--entry-point` and a
  JSON request containing exact typed catalog references and operation-local policy.
- Replace REST `op`, optional `op_version`, optional `config_format`, and flexible TOML/JSON `config`
  with `entry_point` and a JSON `request` object with the same semantics as CLI.
- Change operation discovery to report exact entry-point ids and request schema ids. Do not retain
  accepted-format output.
- Delete direct per-operation TOML/JSON examples and replace them with one setup-import example and
  small exact-reference request examples. Keep runtime-config examples separate.

#### Make provenance local and exact

- Replace `resolved_op_id` plus `entry_point_registry_digest` launch evidence with the exact
  entry-point id and the sorted exact catalog source identities used by that launch.
- Overwrite the current event schema, codec, fixtures, and runtime rendering of launch evidence. Do
  not add a legacy event variant, compatibility decoder, migration shim, or dual-write period.
- Continue to use the generated operation-config digest and certified-spec identity as the
  authority for what executes. Catalog sources explain where reusable inputs came from; they do not
  become another runtime authority.

#### Remove obsolete tests, examples, and dependencies

- Delete tests whose only contract is authored-config parsing, format parity, accepted formats,
  latest-version resolution, old request envelopes, or removed compatibility behavior.
- Rewrite behaviorally valuable app, CLI, REST, operation, and integration tests to seed exact
  catalog values and submit typed-reference requests.
- Delete the old per-operation config examples and the root-level ignored-TOML convention. Keep
  explicit ignores only for genuinely local setup/runtime files.
- Remove `mfm-authored-config` and `mfm-portfolio-config` from every manifest and regenerate the lock
  file. Do not keep facade crates or deprecated re-exports.

The deletion audit estimates roughly 1,000 lines removed with the two config crates, 300--500 lines
from registry/format/latest-resolution and duplicate evidence paths, and 900--1,400 lines of tests
and fixtures that exist only for those paths. Catalog storage, setup import, typed requests, and
builders will add code, but the expected net result is a reduction of roughly 700--1,400 lines and,
more importantly, two fewer crates, one canonicalization path, one launch path, no latest selector,
and substantially fewer public authoring and registry types. These are directional estimates; the
implementation should report measured before/after source and public-type counts.

### What remains intentionally unchanged

The cutover does not move planning into the application or database. It does not move IO into
operations or states. It does not replace certified config artifacts, typed state capabilities,
adapter binding, runtime configuration, replay rules, or higher-level typed operation composition.
Those existing contracts are reused precisely so the new catalog can remain small.

### Explicitly deferred

The initial design does not include:

- separate catalog-entry and revision tables or numeric revisions;
- mutable alias/current-version tables;
- catalog releases, snapshots, lock manifests, or environment promotion;
- granular network, account-set, address-set, token-set, and signer-intent wrappers without a
  demonstrated second consumer;
- nested catalog references, recursive closure, or cycle detection;
- a generic config-builder trait or dynamic resource codec registry;
- a separate `Resolved<T>` abstraction;
- builder identities or content-addressed derivation receipts;
- database-backed runtime profiles;
- catalog workflow, capability, or effect metadata;
- resource-level fleet RBAC, retirement, or garbage collection.

These features may be introduced only in response to a concrete need and must not change the
catalog-to-operation boundary: exact typed values in, complete `MfmConfig` out.

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

The simplified design preserves the essential properties:

| Property | Preserved by |
|---|---|
| Shared authoring | Named reusable typed values written once during setup |
| Strong typing | `CatalogRef<T>`, schema-id verification, strict decode, and `T::validate()` |
| Immutable input | Append-only canonical rows selected by exact digest |
| Deterministic planning | All PostgreSQL access ends before `Operation::expand` |
| Replay independence | Complete generated config and certified spec remain self-contained |
| Secret separation | Only semantic `MfmConfig` values enter this catalog |
| Runtime safety | Existing state capability and adapter/runner binding contracts |
| Workflow composition | Higher-level typed operations compose child operations |
| Minimal provenance | Exact source names, schema ids, and digests in launch evidence |

## Implementation decisions fixed by this RFC

- A small `mfm-config` crate owns `CatalogRef<T>`. It has no PostgreSQL, app, runtime, or operation
  dependency and introduces no catalog service trait.
- The renamed general PostgreSQL storage crate owns the one table and its append, exact-load, list,
  and export implementation. App assembly uses that concrete authority; there is no speculative
  storage abstraction.
- Catalog values use `MfmConfig` directly. A closed setup-import document enum determines which
  concrete resource types users may publish.
- References are always exact. No name-only selector is accepted by setup export, CLI/REST run
  start, preview, or delayed launch.
- The old complete per-operation authored config path is deleted during the run-ingress cutover.
  There is no migration period in which both paths are available.
- The first setup surface is CLI import/list/export. REST setup mutation and an onboarding UI wait
  for an authentication and authorization design; REST run start still consumes exact refs.
- Store/database scope is the catalog authorization and isolation boundary for the first
  implementation. Resource-level RBAC, retention, and audit are deferred rather than approximated.
- Launch evidence stores exact entry-point id plus sorted `(name, schema_id, digest)` sources.
- Kernel, runtime, replay, states, certified configs, and event payload types do not depend on
  `mfm-config` or contain `CatalogRef<T>`; event evidence stores checked scalar source identities.
- Explicit typed schema migrations append new values and are setup-time tools. They are not generic
  launch-time registration or fallback machinery.
- The first higher-level workflow is collect-then-report, proving that shared catalog resources can
  build one composed typed graph while the existing portfolio snapshot entry point remains
  report-only.

Future runtime-profile management, nested values, mutable environment pointers, generic schema
export, REST onboarding, and fleet controls remain separate proposals. They are not blockers for
the implementation described here.

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
