# RFC: Entry-Point Ops As The Only Run Start Surface

Status: proposed.

This RFC defines the intended shape for starting MFM workflows from CLI and REST. It is a breaking
change proposal for the current development branch. We do not need backward compatibility shims,
aliases, or compatibility request bodies for the current domain-specific CLI commands or REST
routes.

This RFC is also a LOC-reduction RFC. The implementation should aggressively remove unused,
unimportant, duplicated, or poorly placed code instead of preserving it behind wrappers. Git history
is the recovery mechanism for deleted code, not compatibility scaffolding in the current tree.

## Summary

MFM should expose one generic run-start surface that starts a certified run from a named
entry-point operation plus minimal authored config:

```sh
mfm_cli run start --op portfolio_snapshot --config portfolio.toml
mfm_cli run start --op evm_contract_lifecycle --config lifecycle.toml
```

REST should expose the same concept through `/v1/runs/start`:

```json
{
  "kind": "typed_run_start_v1",
  "op": "portfolio_snapshot",
  "config": "portfolio_id = \"main\"\n"
}
```

The entry-point op expands deterministically into a typed program draft. That program may be an op
of ops, an op of states, or any mix of nested ops and states. The certifier then emits the
`CertifiedTypedSpec`, app assembly prepares the run launch request, and runtime executes the
certified state machine.

This means the public entrypoint is not `portfolio snapshot`, `evm contracts deploy`, or
`evm contracts lifecycle`. The public entrypoint is `run start --op <NAME>`.

## Motivation

The current CLI and REST surfaces repeat the same execution workflow in domain-specific places:

1. parse a domain request
2. canonicalize or validate config
3. compile an operation into a typed program
4. certify the typed spec
5. gather config and seed launch artifacts
6. prepare a run launch request
7. start the run
8. optionally render public output

This exists today in `mfm_cli portfolio snapshot`, `mfm_cli evm contracts ...`,
`POST /v1/portfolio/snapshot`, and `POST /v1/evm/contracts/*`.

That shape bloats binaries and APIs with per-domain launch wiring. It also contradicts the intended
boundary: CLI and REST should decode transport input and render transport output, while operations
own workflow planning and app/runtime own certified execution.

Another goal is to shrink the codebase by improving reuse. This RFC should remove repeated
portfolio/EVM launch plumbing, request/response types, docs, tests, and helper functions that
exist only because each workflow has its own transport entrypoint. If code is not required by the
new entry-point op path, it should be deleted rather than retained for possible future use.

## Design

### Entry-Point Operation

An entry-point operation is a registered operation that can be launched from public transport
surfaces. It owns:

- its stable typed op id, including namespace and version
- its public name shorthand
- accepted authored config formats
- parsing and canonicalization of minimal authored config
- deterministic expansion into nested ops and states
- program draft construction
- canonical non-secret config and seed material required by the draft
- optional public-output schema discovery

The runtime never executes an op directly. The op is only the deterministic planning ingress. The
certifier mints certified spec authority, app assembly prepares launch authority, and runtime still
executes only a certified typed spec.

The conceptual flow is:

```text
op name/version + authored config
  -> registered entry-point op
  -> canonical op config
  -> typed program draft
  -> certifier mints CertifiedTypedSpec
  -> app assembly prepares RunLaunchRequest
  -> typed state-machine runtime
```

### Registry

CLI and REST need a registry because they receive strings and bytes, not concrete Rust config
types. The registry is type-erasing transport glue. It is not a new workflow layer.

Sketch:

```rust
pub trait LaunchableOp {
    fn op_id(&self) -> EntryPointOpId;
    fn public_name(&self) -> &'static str;
    fn version(&self) -> OpVersion;
    fn accepted_config_formats(&self) -> &'static [ConfigFormat];

    fn plan(
        &self,
        authored_config: AuthoredConfig,
    ) -> Result<EntryPointOpPlan, OpLaunchError>;
}

pub struct EntryPointOpPlan {
    pub draft: TypedProgramDraft,
    pub config_material: Vec<CanonicalConfigMaterial>,
    pub seed_material: Vec<CanonicalSeedMaterial>,
    pub public_output_schema_id: Option<SchemaId>,
    pub lowering_identity: LoweringIdentity,
    pub canonicalizer_identity: CanonicalizerIdentity,
}
```

The registry resolves simple public names to typed op ids. By default, `portfolio_snapshot` resolves
to the latest registered version. Callers can select a specific version when reproducibility or
controlled rollout requires it.

Without this registry, `run start --op portfolio_snapshot` becomes a giant `match` in CLI and REST,
with each new op adding more domain parsing, more dependencies, and more duplicated launch code.
With the registry, CLI and REST remain:

```text
decode op name/version + config
find registered entry-point op
plan to EntryPointOpPlan
certify typed program draft
prepare app-owned run launch
start run
render run response and public output when available
```

The app-facing launch response and `RunAdmitted` evidence must record the submitted public op name,
resolved typed op id and version, entry-point registry digest, lowering identity, canonicalizer
identity, config format, authored config digest, and canonical config digest. This keeps the
`latest` default auditable: two requests that use the same public name at different times may
resolve to different versions, but the admitted run records exactly what was selected.

### CLI Surface

The target CLI surface is:

```sh
mfm_cli run start --op <NAME> --config <PATH> [OPTIONS]
```

Initial options:

- `--op <NAME>`: public op name, such as `portfolio_snapshot`; resolves to the latest version
- `--op-version <VERSION>`: optional explicit op version
- `--config <PATH>`: authored config file
- `--config-format <json|toml>`: optional explicit format; defaults to `toml`
- `--run-id <RUN_ID>`: optional typed run id
- `--drive <append-only|once|until-blocked>`: existing scheduler drive policy

If the run completes during start and the op reports a public output schema id, `run start` renders
that public output automatically in the start response by using the same verified public-output
read-authority path as `run public-output`. It must not render from in-memory op output or from
schema-id presence alone. Callers can still use `run public-output` later to read the stored output
for an existing run.

### REST Surface

The target REST surface is still `POST /v1/runs/start`. It accepts entry-point op starts only.

Entry-point op request:

```json
{
  "kind": "typed_run_start_v1",
  "op": "portfolio_snapshot",
  "config": "portfolio_id = \"main\"\n",
  "run_id": "run:sha256-jcs-v1:<optional-digest>",
  "drive": "until_blocked"
}
```

`config_format` is optional and defaults to `toml` for string configs. JSON object configs imply
`json` unless `config_format` is supplied, in which case the supplied format must be compatible
with the value shape and accepted by the selected op.

JSON-native config is also acceptable for REST:

```json
{
  "kind": "typed_run_start_v1",
  "op": "portfolio_snapshot",
  "op_version": 1,
  "config_format": "json",
  "config": {
    "portfolio_id": "main",
    "wallets": []
  }
}
```

The REST handler must not know portfolio or EVM config structs. It should call the same entry-point
op registry used by CLI. If the run completes during start and the op reports a public output
schema id, the REST response includes rendered public output automatically through the same
verified public-output read-authority path used by `GET /v1/runs/:run_id/public-output/:schema_id`.

### Authored Config Invariants

Entry-point authored config is public ingress data and must be normalized before it can influence a
certified run. Implementations must enforce:

- explicit size limits before parsing
- duplicate-key rejection for object/map-like formats
- unknown-field rejection unless the op config type deliberately marks an extension point
- no floats in any value that reaches hashing or certified config material
- canonical JSON bytes for every config and seed material item
- stable content digests for both authored input bytes and canonicalized config bytes
- redacted parse and validation errors that do not echo secrets, local paths, RPC URLs, or
  authorization material

Authored config must remain non-secret. Secret-bearing runtime mappings stay in process
configuration and capability wiring, not in op config, certified config artifacts, events, public
output, fixtures, or snapshots.

### Portfolio Example

The `portfolio_snapshot` op should accept a small authored config. For example:

```toml
portfolio_id = "main"
quote_codes = ["USD"]

[[wallets]]
wallet_id = "treasury"
address = "0x000000000000000000000000000000000000dead"
network = "ethereum-mainnet"
symbols = ["ETH", "USDC"]

[networks.ethereum-mainnet]
expected_chain_id = 1
```

The op owns the actual topology:

```text
portfolio_snapshot op
  -> optional nested config-normalization ops
  -> source/subject/view states
  -> balance observation states
  -> valuation states
  -> report/snapshot states
  -> public output bindings
```

The authored config should not contain RPC URLs, authorization headers, keystore paths, private
keys, mnemonics, password file paths, or any other runtime-local or secret-bearing data. Runtime
process configuration maps semantic refs such as `ethereum-mainnet` to live sources.

### EVM Contract Example

EVM contract lifecycle entrypoints should also become ops:

- `evm_contract_deploy`
- `evm_contract_configure`
- `evm_contract_validate`
- `evm_contract_lifecycle`

Each op owns its minimal authored config, graph topology, seed requirements, and public-output
schema id. CLI and REST do not expose `evm contracts deploy`, `evm contracts configure`,
`evm contracts validate`, or `evm contracts lifecycle`.

## Required Removals

Because this is a development branch and backward compatibility is not required, implementation
should remove the current domain-specific execution surfaces rather than wrap or deprecate them.
Do not play safe by keeping old code around. The preferred implementation is the smaller one that
preserves the architecture contract and proves behavior through focused tests.

### Remove The Bundle Concept

The current "certified spec bundle" concept must be removed from the system, not merely hidden from
the CLI and REST API. The term "bundle" appears in this RFC only to identify the concept being
removed.

Implementation should remove or rename all user-facing and app-facing bundle surfaces, including:

- CLI flags, help text, docs, and tests that accept or describe bundle input
- REST request fields, examples, docs, and tests that accept or describe bundle input
- app helper APIs whose purpose is parsing or launching from bundle-shaped transport JSON
- public structs, constants, error codes, and rustdoc that name certified spec bundles
- fixtures and integration-test helpers that construct bundle-shaped requests

The replacement is direct entry-point op launch:

```text
op name/version + authored config
  -> EntryPointOpPlan
  -> app-owned certification and launch preparation
  -> RunLaunchRequest
  -> typed runtime
```

Internal certifier/runtime authority may still carry a certified spec and certificate as separate
typed values where required by the authority model, but those values must not be represented,
named, documented, or transported as a "bundle". Because current design and architecture docs use
that terminology, implementing this removal must update those docs in the same change.

Remove CLI surfaces:

- any public run-start mode that bypasses entry-point op selection
- `mfm_cli portfolio snapshot`
- `mfm_cli evm contracts deploy`
- `mfm_cli evm contracts configure`
- `mfm_cli evm contracts validate`
- `mfm_cli evm contracts lifecycle`
- top-level `portfolio` and `evm` command wiring when no remaining non-run command belongs there

Remove REST surfaces:

- any public `/v1/runs/start` request shape that bypasses entry-point op selection
- `POST /v1/portfolio/snapshot`
- `POST /v1/evm/contracts/deploy`
- `POST /v1/evm/contracts/configure`
- `POST /v1/evm/contracts/validate`
- `POST /v1/evm/contracts/lifecycle`

Remove their dedicated request/response structs, route handlers, command modules, tests, and docs.
Do not add compatibility aliases under the old names.

## Documentation Requirements

The implementation must update docs in the same change:

- `bin/cli/README.md`: document `run start --op`, remove any alternate run-start execution mode,
  and remove portfolio and EVM command sections.
- `bin/rest-api/README.md`: document the entry-point op shape under `/v1/runs/start`, remove
  any alternate run-start request shape, and remove domain route sections and endpoint list entries.
- `docs/architecture.md`: keep the rule that CLI/REST are transport-only and reference this RFC
  while the change is in progress.
- `docs/design.md`: update only if the runtime or authority semantics change. The expected design
  should not require runtime semantic changes.
- Any public API rustdoc for new op registry, authored config, or compiled launch types.

Tests that currently assert domain command or route presence must be removed or rewritten to assert
the new generic entry-point op surface.

## Placement

The reusable entry-point op registry should not live in CLI or REST.

Recommended placement:

- app or a small app-adjacent crate owns the registry type and launch preparation glue
- op crates expose deterministic builders, descriptors, and typed planning helpers
- app assembly constructs the production entry-point op registry from those op-crate exports
- CLI and REST receive an already assembled production registry from app assembly
- runtime remains unaware of op ids and authored config formats

The registry may depend on operation crates because it is an app/binary ingress helper. It must not
move operation planning into app assembly. Concrete op crates still own parsing, canonicalization,
and deterministic program expansion for their domain.

## Non-Goals

- Do not expose arbitrary state execution by state name.
- Do not revive old dynamic DAG, `PlannedOp`, context snapshot, or generic feature execution.
- Do not expose a public start path that bypasses entry-point op selection.
- Do not make CLI or REST inspect op internals.
- Do not store secrets, RPC URLs, keystore paths, or password paths in authored config, certified
  config artifacts, events, public output, or test snapshots.
- Do not preserve old EVM and portfolio command or route names.
- Do not treat public route names, command names, or rendered output as runtime authority.

## Decisions

- Entry-point ops have typed ids with namespace and version.
- Public op names are durable API names and must follow the public naming rules in
  `docs/architecture.md`; they are not crate names or temporary recipe labels.
- CLI and REST accept simple public op names by default and resolve them to the latest registered
  version.
- CLI and REST provide an explicit version selector for callers that need a specific op version.
- Public output rendering is automatic when the run completes during start and the op reports a
  public output schema id.
- The effective config format is always known. CLI and REST default to `toml`; callers may select
  another accepted format explicitly.
- Public CLI and REST surfaces expose only entry-point op starts for new run execution.

## Acceptance Criteria

- There is one public CLI execution command for typed runs: `mfm_cli run start`.
- There is one public REST execution route for typed runs: `POST /v1/runs/start`.
- Common workflows are launched by public op name plus minimal config.
- Public op names resolve to the latest registered version unless a version is specified.
- Run evidence records the submitted public name, resolved typed op id/version, registry digest,
  canonicalizer/lowering identity, config format, and authored/canonical config digests.
- Public output is rendered automatically when available at start completion.
- Public CLI and REST docs contain no alternate run-start execution path.
- CLI and REST contain no domain-specific portfolio or EVM launch handlers.
- Runtime still executes only certified typed specs.
- Existing architecture namespace tests continue to prevent old dynamic surfaces from returning.
- New tests prove portfolio and EVM contract flows launch through the generic entry-point op path.
