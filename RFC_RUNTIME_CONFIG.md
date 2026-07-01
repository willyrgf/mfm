# RFC: Runtime Configuration File

## Status

Draft.

## Problem

MFM currently carries live capability wiring across several environment JSON variables:

- `legacy EVM RPC source JSON env`
- `legacy EVM network route JSON env`
- `legacy EVM signer JSON env`

Those variables are awkward because they carry structured configuration in environment values. That
makes local development, review, shell history, Nixfied task wiring, and service deployment harder
than necessary.

The current shape also exposes implementation details at the user boundary. A caller must understand
the split between EVM sources, source policies, semantic network routes, and signer bindings before
they can run a portfolio snapshot or contract lifecycle workflow against a live endpoint. Those
concepts are valid internally, but the primary runtime configuration surface should be a readable
file, not a collection of JSON blobs in the environment.

There is also an inconsistent validation model:

- Portfolio runner setup can treat missing EVM RPC configuration as an unavailable provider.
- Contract lifecycle runner setup validates EVM RPC routing eagerly.

That split makes failures depend on workflow internals instead of a single runtime-configuration
contract. If a workflow requires EVM RPC, missing or invalid runtime wiring should fail consistently,
with redacted diagnostics, before live execution depends on it.

At the same time, RPC endpoints, provider auth, keystore paths, and unlock files must not move into
authored workflow config. Authored `--config` remains semantic run input: it is validated,
certified, hashed into run semantics, replayed, and may influence public artifacts. Runtime process
wiring is local capability configuration and must remain outside typed values, specs, events,
artifacts, and public outputs.

The production run store remains separate process wiring. `DATABASE_URL` or an explicit
`--database-url` continues to select the durable store. Store authority should be established by
store/app service construction, but the store URL is not part of this live capability runtime config
file.

## Goals

- Provide one runtime-only configuration file for live capability bindings.
- Keep authored workflow config semantic and free of endpoints, auth material, keystore paths, and
  other local process resources.
- Keep production run-store selection on the existing `DATABASE_URL` / `--database-url` surface, with
  a store authority guard before constructing store-backed services.
- Make portfolio and contract lifecycle runtime validation consistent.
- Make signer validation effect-scoped: read-only workflows can use public addresses without signer
  runtime bindings, while signing-capable side-effect workflows must have signer bindings.
- Make baseline live capability safety invariants, such as EVM chain identity, mandatory guarded
  capability contracts instead of optional planned validation nodes.
- Keep runtime config out of replay. Replay must use recorded evidence and retained artifacts, not
  live endpoints.
- Make Nixfied managed development tasks pass runtime wiring through the same file surface users can
  inspect and reproduce.
- Support TOML for hand-authored local configuration and JSON for generated or deployment-managed
  configuration from the first implementation.
- Allow runtime-local direct values when users do not want environment indirection, while preserving
  strict redaction and no-persistence rules.

## Non-Goals

- Do not add per-request RPC URL overrides to workflow configs.
- Do not persist runtime endpoints, auth material, database URLs, signer paths, passwords, private
  keys, or signed raw transactions in run streams, artifacts, public outputs, or fixtures.
- Do not make EVM routes part of domain semantics unless selecting a named source is explicitly a
  domain-level requirement in a future workflow.
- Do not preserve environment JSON blobs as a compatibility API.

## Proposed Solution

Introduce a single runtime configuration file for live capabilities and pass it explicitly to live
process entry points.

CLI example:

```sh
mfm run start --op portfolio_snapshot --config portfolio.toml --runtime-config runtime.toml
```

REST example:

```sh
mfm-rest-api --runtime-config runtime.toml
```

If a process manager needs environment-based wiring, the environment should point to the file:

```sh
MFM_RUNTIME_CONFIG_FILE=runtime.toml mfm-rest-api
```

The environment value is only a path. Structured live capability configuration belongs in the file.
Store configuration remains `DATABASE_URL` or `--database-url`.

CLI live start/resume commands should parse runtime config only when the certified run needs live
capabilities from it. When live config is needed, ingress validates the required binding surface for
the whole certified spec. Non-EVM runs must not require an EVM runtime config file.

REST processes should be able to start even when live capability runtime config is missing or
malformed. Read-only services such as status, stream inspection, replay, public-output rendering, and
health should remain available. Live start/resume requests should report redacted deployment errors
when the requested certified run needs a malformed or missing capability family.

## Runtime Config Shape

The runtime config schema is accepted as TOML or JSON. The examples below use TOML.

Common single-source local development example:

```toml
[evm.sources.reth-local]
rpc_url = "http://127.0.0.1:8545"

[evm.routes.reth-dev]
source_ref = "reth-local"
# policy_id is optional here; when no explicit same-id policy exists, it defaults
# to source_ref and synthesizes a same-id single-source policy for the common case.
```

Advanced fallback example:

```toml
[evm.sources.primary-mainnet]
rpc_url_env = "MFM_MAINNET_PRIMARY_RPC_URL"
auth_header_env = "MFM_MAINNET_PRIMARY_AUTH_HEADER"

[evm.sources.secondary-mainnet]
rpc_url_env = "MFM_MAINNET_SECONDARY_RPC_URL"

[evm.policies.ethereum-mainnet]
ordered_sources = ["primary-mainnet", "secondary-mainnet"]

[evm.routes.ethereum-mainnet]
source_ref = "primary-mainnet"
policy_id = "ethereum-mainnet"
```

Signer example:

```toml
[keystores.default]
keystore_path = "/path/to/keystore.json"
unlock_file = "/path/to/password-file"

[signers.deployer]
provider = "keystore"
keystore_ref = "default"
entry_id = "00000000-0000-0000-0000-000000000000"
```

The runtime config may contain direct runtime-local values or point to values through environment or
file indirection. Direct values are useful for local development and single-file process deployment.
Indirection remains recommended for managed secret injection. In all cases, runtime config is a
secret-bearing process-local file: it must not be committed as a fixture, emitted in logs, recorded in
run streams or artifacts, or returned through CLI/REST/public-output surfaces. RPC URLs, RPC auth
values, keystore paths, unlock-file paths, passwords, and private-key material are always redacted in
diagnostics. Private keys, mnemonics, and direct password values must not be represented directly in
this file.

Generated runtime config files must be written outside the repository, preferably under the active
runtime or Nixfied state directory, using restrictive file permissions and atomic write/rename where
the platform supports it. The runtime config file path, indirection file paths, and paths resolved from
environment variables are runtime-local and must be redacted from persisted and public surfaces.

## Semantics

Production store selection is intentionally not part of this runtime config. CLI and REST entry
points continue to obtain the production Postgres connection string from `DATABASE_URL` or
`--database-url`. The resolved URL is secret-bearing and must be redacted. Live start/resume and
evidence-only replay, status, stream inspection, and public-output rendering all need a working store,
but store access is durable authority access rather than live EVM/signer capability wiring.

`evm.sources` defines process-local EVM JSON-RPC endpoints. Source ids are local routing keys. Source
entries do not need an authoritative `expected_chain_id`; the authoritative expected chain id for a
live EVM call comes from the typed capability request guard derived from certified state config or
input. Runtime config entries that include source-level `expected_chain_id` metadata are invalid; MFM
does not preserve the legacy source-chain-id shape as a compatibility surface.

Each EVM source should support direct endpoint/auth values and indirection. Examples include
`rpc_url`, `rpc_url_env`, `rpc_url_file`, `rpc_url_file_env`, `auth_header`, `auth_header_env`,
`auth_header_file`, and `auth_header_file_env`. Resolved endpoint and authorization values are
secret-bearing for redaction purposes, even if a given local endpoint is not sensitive.

`evm.policies` defines ordered fallback across sources. Explicit policies must have non-empty,
duplicate-free source lists. If a route omits `policy_id`, the runtime derives a same-id
single-source policy from `source_ref` for the common case. If an explicit policy with the same id
already exists, omission is rejected; callers must set `policy_id` explicitly to opt into fallback.
Explicit policies should be used when fallback is intended.

`evm.routes` maps semantic workflow `network_id` values to process-local source and policy ids. This
keeps domain network labels separate from local endpoint names. Route keys use semantic network-id
grammar checked through a shared non-secret scalar; app/runtime-config code must not interpret
workflow-specific network semantics. `source_ref` and `policy_id` use local runtime id grammar.
`source_ref` is the preferred starting source within `policy_id`; it must be a member of that
policy's `ordered_sources`. Routes do not carry authoritative chain ids, because chain identity is
semantic workflow input.

`keystores` maps local keystore profile refs to keystore paths and unlock files. `signers` maps
non-secret typed workflow `signer_ref` lookup keys to local signer provider bindings and references
one of those keystore profiles. `signer_ref` is not a provider identity; provider id, keystore entry
id, keystore paths, unlock-file paths, and unlock sources are runtime-local. The config may reference
keystore paths and unlock files directly or through indirection, but those references remain
runtime-only. Direct unlock-file paths are allowed; direct password values are not. Expected public
signer identity remains semantic typed config and non-secret evidence, not runtime config.

## Validation Model

Runtime validation should be shared by all EVM-backed workflows, but it should be layered so generic
runtime-config parsing does not learn workflow semantics.

The responsibility split is:

| Layer | Validates | Must not |
|---|---|---|
| Plan/expand | Deterministic graph topology, semantic requirements, and guard-bearing request shapes | Read runtime config, construct stores/transports/signers, perform live IO, or observe external state |
| Store authority guard | Postgres connectivity, schema/migration compatibility, store metadata, trust-scope setup before store-backed services exist | Become runtime config, workflow semantics, run evidence, public output, or replay capability authority |
| Runtime config shape validation | File syntax, local id grammar, endpoint/auth source shape, route/policy membership, signer provider entry shape | Perform live IO, unlock signers, inspect workflow state, or record evidence |
| Deployment ingress validation | The certified run's required semantic networks and signer refs have local runtime bindings | Call `eth_chainId`, read contract/account state, unlock/sign, append events, or create run status |
| Guarded capability calls | Every live capability call satisfies mandatory request guards using observed response evidence, such as EVM chain identity or signer public identity | Execute live capability guards before `RunAdmitted`, consult replay-time runtime config, or hide workflow topology |
| Semantic validation states | Workflow-domain assertions that need typed outputs, dependencies, facts, or artifacts | Replace mandatory capability guards for baseline safety invariants |
| Management/readiness diagnostics | Operator visibility into current deployment health | Authorize or pre-validate later user runs |

### Guard Timing Rule

A guard runs as early as the authority it protects exists. Plan/expand has only deterministic
planning authority, so it may derive guard requirements and guard-bearing request shapes into the
certified graph, but it must not execute guards or call live providers. Store authority must exist
before `RunAdmitted` can be persisted or read, so the Postgres store authority guard runs during
store construction. Live EVM and signer capability authority exists only inside an admitted node
attempt, so EVM chain guards and signer identity guards run at capability-call time after
`RunAdmitted`.

Moving live EVM guards into plan/expand or deployment ingress would create hidden pre-run live
observations without replayable run evidence, make run existence depend on mutable external RPC
state, and still would not remove the need for the capability-call guard at the real IO boundary.
Readiness commands may perform the same live probes for diagnostics, but those probes never
authorize, reject, or pre-validate a later user run.

### Store Authority Guard

The run store is outside this runtime config file but still must establish authority before any
store-backed service exists. Process entry points that need the durable run store should resolve
`DATABASE_URL` or `--database-url`, connect to Postgres, validate schema compatibility, and load the
store trust scope during app/store service construction.

For live `start` and `resume`, this happens before workflow admission, execution-claim acquisition,
driver loops, or any state-machine attempt event. For evidence-only commands such as status, stream
inspection, replay, and public-output rendering, the same store authority guard happens before
constructing verified run-history or replay/render authority.

The store authority guard is not semantic workflow validation. The database URL is not hashed into run
semantics, not recorded in `RunAdmitted`, and not part of replay authority.

The store authority guard may use the same shape as live capability guards, but it is a storage
authority contract rather than a workflow capability guard. A Postgres store authority guard should
verify deployment facts such as:

- the database connection can be established;
- the schema and migration set are compatible with the compiled store implementation;
- required metadata tables exist and expose a valid store-owned trust scope;
- the trust scope can be initialized only through the store-owned first-boot path, never from runtime
  config or workflow input.

The resulting guard transcript is process diagnostic authority only. It may be used to construct an
in-process verified store handle, but it must not be written into run streams, retained as workflow
evidence, used by replay, or exposed with database URLs or other secret-bearing connection details.
The store-owned `trust_scope_id` loaded from metadata is different: it is non-secret deployment
identity material that already participates in `RunIdentityMaterialV1` and is recorded through
`RunAdmitted` as part of run identity. Runtime config and workflow config must never supply or mutate
that trust scope.

The storage API may mirror the guard shape without reusing workflow capability traits:

```rust
pub trait StoreAuthorityGuard {
    type Evidence;

    fn verify(&self, evidence: &Self::Evidence) -> Result<(), StoreAuthorityError>;
}

pub struct PostgresStoreAuthorityRequirement {
    pub schema: PostgresSchemaRequirement,
    pub trust_scope: StoreTrustScopeRequirement,
}

pub struct PostgresStoreAuthorityEvidence {
    pub schema_version: StoreSchemaVersion,
    pub migration_set_digest: ContentDigest,
    pub trust_scope_id: StoreTrustScopeId,
}
```

`PostgresStoreAuthorityRequirement` is supplied by the compiled store/app code, not by runtime config
or workflow config. `PostgresStoreAuthorityEvidence` is an in-process verification object, not
workflow evidence. It must remain outside certified specs, retained artifacts, and public outputs, and
must not be serialized into run events as a guard record. Its `trust_scope_id` field may be copied only
through the existing store-owned run identity path described above.

### Runtime Config Shape Validation

Runtime config shape validation checks deployment-local shape. For CLI live commands, this should run
only when the certified run requires live capabilities from the file. For REST, missing or malformed
live runtime config must not fail process startup or read-only services; it becomes a redacted
deployment error only when a live start/resume request needs that capability family. When an EVM or
signer section is needed by a live run, the section is validated as a whole; an unused malformed entry
in that capability family still blocks the live run. Evidence-only status, stream inspection, replay,
and public-output paths never parse or validate live runtime config.

Runtime config parsing and value-source resolution belong together. The runtime-config crate should
parse TOML/JSON, resolve direct/env/file indirections into redacted runtime-local descriptors, and
validate deployment-local shape. It must not construct stores, transports, signer providers, unlock
keystores, call RPC endpoints, or execute readiness probes.

When a live capability section is parsed, the runtime config validator should check:

- every EVM source id and policy id uses the checked local runtime-id grammar;
- every signer ref uses the checked signer-reference grammar;
- every route key uses checked semantic network-id grammar without workflow-specific interpretation;
- every EVM source has exactly one endpoint source (`rpc_url`, `rpc_url_env`, or file indirection);
- direct RPC URLs are syntactically valid and do not contain URL userinfo;
- RPC authorization has at most one source, whether direct or indirect;
- every explicit policy has at least one unique source;
- every policy source exists;
- every route references an existing source and policy;
- every route `source_ref` is a member of the selected policy;
- every signer entry has a supported provider shape and checked local sources;
- signer entries do not contain private keys, mnemonics, passwords, password values, or raw signing
  material.

All parse and validation diagnostics must be closed-category and redacted. It is acceptable to name
non-secret ids such as `network_id`, `source_ref`, `policy_id`, and `signer_ref`; diagnostics must
not include runtime config paths, RPC URLs, auth material, database URLs, passwords, private keys,
keystore paths, password paths, indirection file paths, env-resolved paths, or signed transactions,
regardless of whether those values came directly from the file or from indirection.

### Deployment Ingress Validation

Before live execution is admitted or resumed, adapter-owned runner ingress should derive deployment
binding requirements from the whole certified spec and its config artifacts:

- EVM read requirements: `(network_id, expected_chain_id)` for portfolio reads and contract
  validation reads.
- EVM mutation requirements: `(network_id, expected_chain_id, signer_ref)` for signing-capable
  side-effect nodes such as contract deploy/configure submit nodes.

The `expected_chain_id` in these requirements identifies the guard the live provider must enforce. It
is not compared to live RPC state, and it is not matched against authoritative runtime-config chain
truth.

The runtime/app layer should invoke these runner-owned ingress checks rather than embedding workflow
semantics in app code. Shared helpers may validate common route/policy/source and signer-binding
rules, but adapters own requirement extraction. The checks should cover:

- every required semantic network has a runtime route;
- every EVM mutation requirement has a configured signer provider for its `signer_ref`.

Deterministic semantic consistency, such as rejecting one certified workflow that requires the same
`network_id` with conflicting `expected_chain_id` values, belongs in plan/expand or certification, not
deployment ingress. Ingress may rely on that prior deterministic validation and must not become a
hidden semantic validator.

Deployment ingress validation is intentionally not a workflow. It appends no events, records no
artifacts, does not acquire execution claims, and does not produce run status or public output. For
new live starts, it runs before `RunAdmitted`; failures reject launch and leave no run stream. For
live resume/drive, including resume after manual-resolution recording, it validates the whole stored
certified spec after the stored certified spec and retained config artifacts have been verified and
stored binding compatibility has been checked, but before execution-claim acquisition, state-machine
transition selection, runner invocation, or any attempt event append. This whole-spec requirement is
intentional for live drive authority; evidence-only status, stream inspection, replay, and
public-output rendering remain independent of live runtime config validity.

Deployment ingress validation must not perform live semantic observations. It should not call
`eth_chainId`, unlock keystores, sign challenge material, inspect contract state, read account state,
or probe private key material. It checks that the process has the deployment bindings needed for the
certified run. It must not create a parallel workflow planner, make runtime config part of run
identity, or produce replay authority. Runtime config contents, paths, env names, route/source/policy
ids, signer entries, endpoints, and auth material must not enter `RunIdentityMaterialV1` or
`RunAdmitted.admitted_binding_digest`; that digest remains limited to registered executable,
adapter, runner, and capability implementation identities.

Signer validation is effect-scoped. Public addresses in read-only portfolio or validation configs
are query subjects, not signer authority, and must not require `[signers]`. For mutation
workflows, ingress validation verifies that a signer binding exists. It should not unlock keystores
or probe private key material during admission. The actual expected public identity check happens when
the signing request is materialized: typed config supplies the expected public identity, the signer
result verifies that identity, and raw signed transaction bytes remain transient.

Portfolio and contract lifecycle should therefore fail the same way for missing EVM runtime wiring.
Non-EVM portfolio configs should not require EVM runtime configuration, and read-only EVM portfolio
configs should not require signer runtime configuration.

Portfolio runners should implement runner-ingress validation for EVM read requirements instead of
accepting an unavailable provider until first use. Contract lifecycle runners should keep validating
read-only phases without signer bindings and mutation phases with signer bindings.

Adapters and transports should still validate defensively when used, but the primary user-facing
failure for missing deployment wiring should come from ingress validation before live state-machine
drive.

### Guarded Capability Calls

Baseline safety invariants for live capabilities should be expressed as required guards on capability
requests. A guard is semantic, non-secret request data derived from certified state config or typed
input. The live provider must verify the guard before the protected live operation and return redacted
guard evidence with the response. Replay providers verify recorded guard evidence and never call live
IO.

The generic public API should stay small:

```rust
pub trait CapabilityGuard: Clone + Eq + Send + Sync + 'static {
    type Evidence: Clone + Eq + Send + Sync + 'static;

    fn verify(&self, evidence: &Self::Evidence) -> Result<(), GuardViolation>;
}

pub trait GuardedCapabilityRequest {
    type Guard: CapabilityGuard;

    fn guard(&self) -> &Self::Guard;
}

pub trait GuardedCapabilityResponse<R: GuardedCapabilityRequest> {
    fn guard_evidence(&self) -> &<R::Guard as CapabilityGuard>::Evidence;
}
```

These traits are a compile-time capability-contract helper, not a new runtime authority registry. If
multiple capability families use them, they may live in `mfm-capabilities`; otherwise each capability
contract crate may own equivalent local traits until the abstraction proves useful.

For EVM, every live EVM request should carry an EVM chain guard:

```rust
pub struct EvmChainGuard {
    pub network_id: EvmNetworkId,
    pub expected_chain_id: NonZeroU64,
}

pub struct EvmChainGuardEvidence {
    pub network_id: EvmNetworkId,
    pub expected_chain_id: NonZeroU64,
    pub observed_chain_id: NonZeroU64,
    pub selected_source_ref: EvmSourceRef,
    pub policy_id: EvmSourcePolicyId,
}
```

EVM capability requests should be semantic-first and should not expose unguarded equivalents:

```rust
pub struct EvmCallReadRequest {
    pub chain: EvmChainGuard,
    pub to: Address,
    pub calldata: Vec<u8>,
    pub block: EvmBlockSelector,
}
```

The adapter derives `EvmChainGuard` from certified state config and input, then asks the live provider
to execute the request. The live EVM transport maps `network_id` to runtime-local route/source/policy,
calls `eth_chainId`, verifies `observed_chain_id == expected_chain_id`, and only then performs the
requested read, estimate, receipt lookup, nonce read, fee read, or transaction submission. For
mutation flows, the guard check must happen before nonce reads, fee/gas work, signing, and submit.

Plan/expand may make the need for this guard explicit by producing guard-bearing state/request
shapes, and deployment ingress may verify that the semantic `network_id` has a local route. Neither
stage executes the EVM guard. The first authoritative live EVM guard check happens inside the live
provider for the admitted capability call.

Guard failure is a post-admission capability/state-attempt failure with redacted diagnostics, not a
deployment ingress failure. It follows the normal attempt failure and side-effect evidence rules for
the effect class. Replay and status use recorded stream evidence only and must never re-run a guard
against live runtime config. Guard evidence may include redacted runtime provenance such as selected
source and policy ids, but those ids are not semantic authority and replay must not resolve them
against current runtime config.

Guard failures that observe live evidence must carry a closed, redacted failure-evidence shape rather
than an ad hoc error string. For an EVM chain mismatch, failure evidence should include the semantic
`network_id`, semantic expected chain id, observed chain id when available, selected source ref, and
policy id, but no endpoint, auth, path, request payload secret, or provider diagnostic body. That
failure evidence is recorded only through the normal post-admission attempt/side-effect evidence path,
and replay verifies it against the request guard without constructing live runtime config.

Signing already follows the same shape: typed config supplies `signer_ref` plus expected public
identity, the signing request requires that public identity, and the signer result is verified before
transient raw signed bytes are materialized. Do not add a separate pre-admission signing challenge for
this baseline guard.

This design must remove several older validation structures:

- no mandatory operation-planned "validate EVM chain identity" node at the start of every EVM workflow;
- no pre-`RunAdmitted` live `eth_chainId` readiness gate for user runs;
- no app-owned workflow semantic validator that interprets portfolio or contract lifecycle config;
- no adapter-local route tables whose only purpose is to turn `network_id` into `source_ref` and
  `policy_id` before building every EVM request;
- no state/adapter-facing EVM capability request fields for `source_ref` or `policy_id`; route
  selection is provider-local and appears only as redacted response evidence;
- no required source-level `expected_chain_id` in runtime config; semantic chain identity comes from
  the request guard;
- no repeated per-adapter "call chain_identity, compare expected_chain_id, then do the real request"
  prelude. That behavior should collapse into the guarded EVM provider/transport layer, with adapters
  deriving guards and defensively verifying returned guard evidence.

Other guard-like checks should use the same rule when they are local to a capability request/response:

- artifact reads should continue verifying requested artifact id, digest, schema id, semantic type id,
  producer evidence, role, byte length, and bytes-to-digest coherence before returning bytes;
- EVM block, balance, call, log, nonce, fee, gas, receipt, and submit responses should all carry and
  verify EVM chain guard evidence;
- EVM responses with request-specific identity should verify that identity before returning, such as a
  block response matching the requested block hash/number, logs falling within the requested filter,
  receipts matching the requested transaction hash, and transaction submit evidence matching the
  submitted raw transaction hash;
- signing providers should continue requiring algorithm/domain/purpose/digest and expected public
  identity checks before returning a usable signature.

Do not add guard abstractions for checks that need workflow topology, cross-state dependency, or
domain-specific user-visible conclusions. Those remain explicit semantic validation states.

### Semantic Capability Validation

Workflow-domain validation that observes live external state and affects workflow correctness belongs
inside the certified user run as ordinary typed states. Operations may plan those validation states
before dependent work, but operations must not execute live validation themselves. States own the
validation semantics, adapters bind state intent to explicit capabilities, and transports/signers
perform the live protocol work.

Semantic validation states are separate from baseline capability guards. Use guards for invariants
that must hold for every capability call, and use explicit states when the workflow needs a typed,
queryable, dependency-bearing validation result.

Examples of semantic capability validation include:

- explicit chain-identity read states when the workflow needs chain identity as a typed domain fact,
  beyond the mandatory EVM chain guard on each capability call;
- deployed bytecode or contract-call checks;
- account, nonce, balance, log, receipt, or contract-state reads;
- signer public-identity checks if a future read-only signer identity capability is introduced.

Those states run after `RunAdmitted`, record typed facts or artifacts, and replay from recorded
evidence only. If a validation result blocks downstream work, the dependency should be represented in
the certified graph so downstream states cannot execute without the validation output. A failed
semantic validation is a normal post-admission state outcome with redacted diagnostics, not a launch
ingress failure.

Live `eth_chainId` checks performed by transports as readiness diagnostics are operational telemetry
only. Live `eth_chainId` checks performed as mandatory EVM chain guards happen inside certified
capability calls after admission. They are baseline safety evidence for that capability call, not a
separate validation workflow. If observed chain identity is workflow-domain evidence, it must also be
represented through a certified read state and recorded evidence.

### Management Validation Runs

A separate management or readiness workflow may be useful for operator diagnostics, but it cannot
authorize or replace validation in a later user run. Its result is non-authoritative for the later run
because runtime-observed values cannot mutate the topology or semantics of a separately certified
run, and the live world may change between readiness and launch. If management validation is added,
it should be presented as diagnostics/readiness, not as hidden pre-admission authority.

## Evidence and Replay

Replay must not load live capability runtime config, construct live transports, call live signers, or
inspect keystores. Replay still connects to the durable run store through `DATABASE_URL` or
`--database-url`, then verifies from certified spec/certificate artifacts, the append-only run stream,
retained artifacts, and recorded read/side-effect evidence.

Status, stream inspection, replay, and public-output rendering services must be constructible without
valid live runtime capability configuration. A malformed or missing runtime config may block live
start/resume/drive when the whole certified spec needs that capability family, but it must not block
evidence-only read or replay paths. App assembly should keep evidence-only service construction free of
live runtime config parsing, live transports, and signer providers.

Recorded EVM evidence may include redacted `selected_source_ref`, `policy_id`, observed `chain_id`,
semantic `network_id`, and semantic expected chain id as audit provenance. `selected_source_ref`
means the actual source that served the call after policy fallback, not merely the route's preferred
starting source. These runtime refs are not consistency, retry, replay, or public-output authority:
replay must not resolve them against the current runtime config, and retry behavior must not depend
on them. Live providers and replay providers must verify EVM chain guard evidence against the request
guard before returning capability responses. Adapters that build typed outputs from EVM reads must
defensively validate observed chain id against semantic expected chain id before producing output,
including portfolio pin/read paths.

## Architecture Placement

The runtime config parser should live in a small dedicated runtime-config crate. It should not live in
workflow operation crates or state crates.

Suggested ownership:

- `mfm-capabilities`: optionally own tiny generic guard traits if more than one capability family uses
  them. It must not own domain guard semantics, live IO, runtime route resolution, or a mutable guard
  registry.
- `mfm-runtime-config`: parse live capability runtime configuration, resolve direct/env/file
  indirections into redacted runtime-local descriptors, and validate deployment-local shape. Keep it to
  schema parsing, value-source resolution, redacted diagnostics, and reduced typed descriptors; it must
  not construct stores, transports, signers, app services, runners, or replay services, and it must not
  unlock keystores or call live providers.
- `mfm-transports-evm`: construct clients from explicit typed source registries; no direct env
  parsing in the primary path; map semantic EVM chain guards to runtime-local route/source/policy
  bindings supplied by app assembly; validate route/policy membership without workflow semantics.
  Live chain-id checks belong inside guarded certified capability calls or explicit diagnostic
  readiness commands, not runtime-config parsing or deployment ingress.
- `mfm-adapters-*`: receive explicit capability providers and signer providers; derive
  workflow-specific deployment binding requirements in runner ingress from certified nodes and config
  artifacts; derive capability guards from certified state config/input for each guarded request;
  defensively verify guard evidence; bind semantic validation state intent to live/replay capability
  providers. Adapters should not own EVM route tables.
- `mfm-signers-*`: construct signer providers from explicit typed signer registries; verify expected
  public identities at signing time without leaking secret-bearing details.
- `mfm-storages-stream-store-postgres`: own the Postgres store authority guard for connection,
  migration/schema compatibility, and store metadata/trust-scope setup. These checks establish
  storage authority, not live capability authority, runtime config authority, or run evidence.
- `mfm-app`: construct the production store from `DATABASE_URL` / `--database-url`, construct live
  transports and signer providers from parsed runtime config only for live-capability services,
  construct store-backed services only from a store handle that passed the store authority guard,
  invoke adapter-owned deployment ingress validation before live drive, and keep evidence-only
  service assembly independent from live runtime capability validity.
- binaries: for live-capability commands and services, read the CLI/env file path and pass the
  runtime config source or parsed runtime descriptors into live app assembly. REST startup must not
  depend on parsing live runtime config successfully.
- Nixfied: generate or pass a live capability runtime config file for managed dev services.

## Migration Direction

Because MFM is pre-production, correctness and clarity should take priority over preserving awkward
environment JSON APIs.

Recommended migration:

1. Introduce typed runtime config parsing, value-source resolution, and validation for both TOML and
   JSON in `mfm-runtime-config`, with redacted diagnostics and runtime-only value-source types.
2. Split app assembly so evidence-only status, stream, replay, and public-output services do not
   parse live runtime config, construct live transports, or construct signer providers.
3. Add `--runtime-config <PATH>` to `mfm run start` and `mfm run resume`.
4. Add REST runtime config file support without making server startup depend on live capability
   config validity; live start/resume requests fail when they need a malformed or missing capability
   family.
5. Keep `--database-url` and `DATABASE_URL` as the production store surface, with the store authority
   guard in app/store service construction.
6. Introduce guarded capability request/response contracts for EVM chain identity. Update EVM
   capability requests so `network_id` and semantic expected chain id come from required request
   guards, not from runtime source metadata.
7. Update EVM transports to resolve guarded semantic network requests to runtime-local routes and to
   verify `eth_chainId` against the request guard before each live EVM operation, including mutation
   preparation before nonce, fee/gas, signing, and submit.
8. Update adapters to derive EVM chain guards from certified state config/input and to defensively
   verify returned guard evidence before producing typed outputs or side-effect evidence.
9. Introduce a sibling Postgres store authority guard/check contract for connection,
   migration/schema compatibility, and store metadata/trust-scope setup. Keep it outside runtime
   config, certified specs, run streams, replay, and public output.
10. Remove source-level `expected_chain_id` from the canonical runtime config shape and reject legacy
   source chain metadata instead of accepting it as compatibility input. Request guards remain
   authoritative.
11. Remove adapter-local EVM route wrappers whose only job is to inject `source_ref` and `policy_id`
    into capability requests. Route resolution belongs in the guarded provider/transport, and selected
    route ids appear only as redacted evidence.
12. Remove existing `legacy EVM RPC sources flag` and EVM environment JSON compatibility inputs rather than
   normalizing them as compatibility shims. Direct values and indirect sources should resolve only
   through the new redacted runtime value types.
13. Delete the old EVM runtime wiring surfaces: env constants, direct reads of
   `legacy EVM RPC source JSON env`, `legacy EVM network route JSON env`, and `legacy EVM signer JSON env`
   constructors, source-level runtime `expected_chain_id` fields/parsers/tests, and old route-table
   request plumbing that exposes `source_ref` or `policy_id` to adapters/states.
14. Update Nixfied Reth workflows to generate or pass live capability runtime config files. Generated
   files should be written outside the repository under the active runtime/Nixfied state directory with
   restrictive permissions, atomic write/rename where supported, and secret-bearing file handling.
15. Add tests that distinguish deployment ingress from guarded capability failures and semantic
   validation: ingress failures append no `RunAdmitted`, guarded EVM chain mismatches fail after
   admission as capability/state-attempt failures with replayable redacted evidence, semantic
   validation records replayable evidence after admission, and replay succeeds with live runtime
   config path/file, signer/RPC environment, and referenced secret/value files removed.
16. Add tests that malformed or missing live runtime config does not block REST startup, status,
    stream inspection, replay, or public-output rendering, while live start/resume reports redacted
    deployment errors when the certified run needs a malformed or missing capability family.
17. Add store authority guard tests for unavailable Postgres, migration/schema incompatibility,
    metadata/trust-scope setup, and redaction of database URLs.
18. Update docs and tests to use `runtime.toml` or `runtime.json` for EVM/signer capability wiring.
19. Remove or rewrite docs that describe the old EVM runtime JSON surfaces, including
   `docs/evm-rpc-routing.md`, CLI README sections, REST README sections, fixtures, and integration
   test setup.

Temporary compatibility shims should not be added. MFM is pre-production; remove flawed live runtime
configuration surfaces deliberately and update docs/tests in the same change.

## Decisions

- REST processes start normally when live capability runtime config is missing or malformed.
  Capability-specific errors are reported when live start/resume requests need the malformed or missing
  capability family.
- Live start/resume deployment ingress validates live binding availability for the whole certified
  spec. Evidence-only status, stream inspection, replay, and public-output rendering remain independent
  of live runtime config validity.
- Runtime config parsing, direct/env/file value-source resolution, and shape validation live in
  `mfm-runtime-config`; that crate must not construct stores, transports, signers, app services,
  runners, or replay services.
- MFM does not preserve compatibility for legacy EVM runtime JSON blobs, `legacy EVM RPC sources flag`, or
  source-level `expected_chain_id`; those surfaces are removed or rejected.
- Environment variable names are runtime-local selectors, not secret material by themselves. Resolved
  values, paths, URLs, authorization headers, passwords, private keys, and signed material remain
  secret-bearing and redacted.
- TOML and JSON are both supported from the first implementation.
- Direct keystore paths and unlock-file paths are allowed runtime-local values. They are not limited
  to local development, but they remain forbidden in persisted, public, fixture, and replay surfaces.
- EVM chain identity is a mandatory guarded capability invariant. The canonical runtime config shape
  does not need source-level `expected_chain_id`; the authoritative expected chain id for a live EVM
  call comes from the certified request guard.
- Postgres connection, migration/schema, and trust-scope checks use a sibling store authority guard
  pattern. They are not part of live capability runtime config, not certified workflow semantics, and
  not replay authority. The guard transcript is not persisted as run evidence, while the store-owned
  non-secret `trust_scope_id` continues to enter `RunIdentityMaterialV1` through the existing run
  identity path.

## Open Questions

None currently.
