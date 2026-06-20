# RFC: Entry-Point Ops As The Only Run Start Surface

Status: proposed.

This RFC defines the intended shape for starting MFM workflows from CLI and REST. It is a breaking
change proposal for the current development branch. We do not need backward compatibility shims,
aliases, or compatibility request bodies for the current domain-specific CLI commands or REST
routes.

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
  "config_format": "toml",
  "config": "portfolio_id = \"main\"\n"
}
```

The entry-point op expands deterministically into a typed program. That program may be an op of
ops, an op of states, or any mix of nested ops and states. The certifier then emits the
`CertifiedTypedSpec`, the app prepares launch material, and runtime executes the certified state
machine.

This means the public entrypoint is not `portfolio snapshot`, `evm contracts deploy`, or
`evm contracts lifecycle`. The public entrypoint is `run start --op <OP_ID>`.

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

## Design

### Entry-Point Operation

An entry-point operation is a registered operation that can be launched from public transport
surfaces. It owns:

- its stable op id
- accepted authored config formats
- parsing and canonicalization of minimal authored config
- deterministic expansion into nested ops and states
- certification input preparation
- launch config and seed artifact materialization
- optional public-output schema discovery

The runtime never executes an op directly. The op is only the planning and certification ingress.
Runtime still executes only a certified typed spec.

The conceptual flow is:

```text
op id + authored config
  -> registered entry-point op
  -> canonical op config
  -> typed program draft
  -> CertifiedTypedSpec
  -> RunLaunchRequest
  -> typed state-machine runtime
```

### Registry

CLI and REST need a registry because they receive strings and bytes, not concrete Rust config
types. The registry is type-erasing transport glue. It is not a new workflow layer.

Sketch:

```rust
pub trait LaunchableOp {
    fn op_id(&self) -> OpId;
    fn accepted_config_formats(&self) -> &'static [ConfigFormat];

    fn compile(
        &self,
        authored_config: AuthoredConfig,
    ) -> Result<CompiledOpRun, OpLaunchError>;
}

pub struct CompiledOpRun {
    pub certified_spec: CertifiedTypedSpec,
    pub config_artifacts: Vec<RunLaunchConfigArtifact>,
    pub seed_artifacts: Vec<RunLaunchSeedArtifact>,
    pub public_output_schema_id: Option<SchemaId>,
}
```

Without this registry, `run start --op portfolio_snapshot` becomes a giant `match` in CLI and REST,
with each new op adding more domain parsing, more dependencies, and more duplicated launch code.
With the registry, CLI and REST remain:

```text
decode op id + config
find registered entry-point op
compile to CompiledOpRun
prepare certified launch
start run
render response
```

### CLI Surface

The target CLI surface is:

```sh
mfm_cli run start --op <OP_ID> --config <PATH> [OPTIONS]
```

Initial options:

- `--op <OP_ID>`: stable entry-point op id, such as `portfolio_snapshot`
- `--config <PATH>`: authored config file
- `--config-format <json|toml>`: optional explicit format; defaults from extension when possible
- `--run-id <RUN_ID>`: optional typed run id
- `--drive <append-only|once|until-blocked>`: existing scheduler drive policy
- `--render-public-output`: if the run completes and the op reports a public output schema id,
  render it in the start response

The existing low-level certified bundle launch can remain as a separate advanced mode:

```sh
mfm_cli run start --bundle <PATH> [--config <SCHEMA_ID=PATH>]... [--seed <SEED_ID=PATH>]...
```

It is useful for reproducibility, fixtures, and explicitly supplied certified specs. It is not the
primary user-facing path for common workflows.

### REST Surface

The target REST surface is still `POST /v1/runs/start`. It should accept either an entry-point op
request or the existing low-level certified bundle request.

Entry-point op request:

```json
{
  "kind": "typed_run_start_v1",
  "op": "portfolio_snapshot",
  "config_format": "toml",
  "config": "portfolio_id = \"main\"\n",
  "run_id": "run:sha256-jcs-v1:<optional-digest>",
  "drive": "until_blocked",
  "render_public_output": true
}
```

JSON-native config is also acceptable for REST:

```json
{
  "kind": "typed_run_start_v1",
  "op": "portfolio_snapshot",
  "config_format": "json",
  "config": {
    "portfolio_id": "main",
    "wallets": []
  }
}
```

The REST handler must not know portfolio or EVM config structs. It should call the same entry-point
op registry used by CLI.

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

Remove CLI surfaces:

- `mfm_cli portfolio snapshot`
- `mfm_cli evm contracts deploy`
- `mfm_cli evm contracts configure`
- `mfm_cli evm contracts validate`
- `mfm_cli evm contracts lifecycle`
- top-level `portfolio` and `evm` command wiring when no remaining non-run command belongs there

Remove REST surfaces:

- `POST /v1/portfolio/snapshot`
- `POST /v1/evm/contracts/deploy`
- `POST /v1/evm/contracts/configure`
- `POST /v1/evm/contracts/validate`
- `POST /v1/evm/contracts/lifecycle`

Remove their dedicated request/response structs, route handlers, command modules, tests, and docs.
Do not add compatibility aliases under the old names.

## Documentation Requirements

The implementation must update docs in the same change:

- `bin/cli/README.md`: document `run start --op`, remove portfolio and EVM command sections, and
  keep low-level `--bundle` launch as an advanced mode if retained.
- `bin/rest-api/README.md`: document the entry-point op shape under `/v1/runs/start`, remove
  domain route sections and endpoint list entries.
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
- op crates register launchable entry-point ops
- CLI and REST receive an already assembled production registry from app assembly
- runtime remains unaware of op ids and authored config formats

The registry may depend on operation crates because it is an app/binary ingress helper. It must not
move operation planning into app assembly. Concrete op crates still own parsing, canonicalization,
and deterministic program expansion for their domain.

## Non-Goals

- Do not expose arbitrary state execution by state name.
- Do not revive old dynamic DAG, `PlannedOp`, context snapshot, or generic feature execution.
- Do not make CLI or REST inspect op internals.
- Do not store secrets, RPC URLs, keystore paths, or password paths in authored config, certified
  config artifacts, events, public output, or test snapshots.
- Do not preserve old EVM and portfolio command or route names.
- Do not treat public route names, command names, or rendered output as runtime authority.

## Open Questions

- Should entry-point op ids use bare names like `portfolio_snapshot` or typed ids with namespace
  and version?
- Should `--render-public-output` be opt-in only, or should run start render automatically when the
  op reports a public schema id and the run completes?
- Should `--config-format` be required for REST string configs, or can content type / request shape
  select it?
- Should low-level `--bundle` launch stay on `run start` or move to `run start --certified-bundle`
  for clearer help output?

## Acceptance Criteria

- There is one public CLI execution command for typed runs: `mfm_cli run start`.
- There is one public REST execution route for typed runs: `POST /v1/runs/start`.
- Common workflows are launched by op id plus minimal config.
- CLI and REST contain no domain-specific portfolio or EVM launch handlers.
- Runtime still executes only certified typed specs.
- Existing architecture namespace tests continue to prevent old dynamic surfaces from returning.
- New tests prove portfolio and EVM contract flows launch through the generic entry-point op path.
