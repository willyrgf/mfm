# Implementation Plan: RFC Entry-Point Ops

Status: planning.

This plan implements `docs/RFC_ENTRYPOINT_OP.md`.

## Non-Negotiables

- Do not maintain backward compatibility for removed CLI commands, REST routes, request bodies,
  response shapes, flags, error codes, docs, or tests.
- Breaking changes are allowed and expected.
- Do not create fallbacks, aliases, compatibility wrappers, deprecation paths, or hidden old-code
  dispatch.
- Delete old code when the entry-point op path replaces it. Git history is the recovery mechanism.
- Keep each commit focused and reviewable. Do not mix unrelated cleanup into a commit.
- If an architectural decision becomes unclear, pause that commit, spawn an independent
  architect-agent, pass it this plan plus `docs/RFC_ENTRYPOINT_OP.md`, and resolve the decision
  before coding further.

## Target End State

Public execution ingress:

```sh
mfm_cli run start --op portfolio_snapshot --config portfolio.toml
mfm_cli run start --op evm_contract_lifecycle --config lifecycle.toml --op-version 1
```

```http
POST /v1/runs/start
```

with a body shaped around:

```json
{
  "kind": "typed_run_start_v1",
  "op": "portfolio_snapshot",
  "op_version": 1,
  "config_format": "toml",
  "config": "portfolio_id = \"main\"\n"
}
```

Deleted public execution ingress:

- `mfm_cli portfolio snapshot`
- `mfm_cli evm contracts deploy`
- `mfm_cli evm contracts configure`
- `mfm_cli evm contracts validate`
- `mfm_cli evm contracts lifecycle`
- public `run start` modes that bypass entry-point op selection
- `POST /v1/portfolio/snapshot`
- `POST /v1/evm/contracts/deploy`
- `POST /v1/evm/contracts/configure`
- `POST /v1/evm/contracts/validate`
- `POST /v1/evm/contracts/lifecycle`
- public bundle-shaped launch APIs, helpers, docs, tests, and fixtures

## Commit Plan

### Commit 1: Add Guardrails And Failing Contract Tests

Goal: make the desired deletion and replacement explicit before implementation.

Changes:

- Add or update architecture namespace tests to reject:
  - `mfm_cli portfolio snapshot`
  - `mfm_cli evm contracts ...`
  - REST `/v1/portfolio/snapshot`
  - REST `/v1/evm/contracts/*`
  - public `bundle` launch naming in CLI/REST/app-facing APIs
- Add CLI parse/help tests for the new intended surface:
  - `run start --op <NAME> --config <PATH>`
  - `--op-version`
  - `--config-format`
- Add REST request-shape tests for `/v1/runs/start` with `op`, optional `op_version`, default TOML
  config format, and JSON config format.

Expected result:

- Tests fail because implementation is not present yet.
- Failures should be narrow and describe the intended API.

Verification:

```sh
cargo test -p mfm --test cli_tests
cargo test -p mfm-integration-tests --test rest_api_run_control
cargo test -p mfm-integration-tests --test architecture_namespace_contract
```

### Commit 2: Introduce Entry-Point Op Registry Types

Goal: create the reusable dispatch abstraction outside CLI and REST.

Preferred placement:

- `crates/app` if the surface is small and clearly app-ingress-only
- or a small app-adjacent crate if `crates/app` would become too broad

Types to add:

- `EntryPointOpId`: typed id with namespace and version
- `PublicOpName`
- `OpVersion`
- `ConfigFormat`
- `AuthoredConfig`
- `EntryPointOpPlan`
- `LaunchableOp`
- `EntryPointOpRegistry`
- `EntryPointOpResolveError` / `OpLaunchError`

Boundary requirements:

- `LaunchableOp::plan` returns a typed program draft and canonical non-secret material.
- It must not return `CertifiedTypedSpec`.
- It must not connect stores, stage artifacts, launch runs, render outputs, read environment
  variables, or create runtime capabilities.
- App assembly owns certification and launch preparation after planning.

Verification:

```sh
cargo test -p mfm-app entry_point
cargo check -p mfm-app
```

### Commit 3: Add Authored Config Normalization

Goal: make config intake reusable and safe before any op uses it.

Implement:

- TOML default config format.
- JSON config format.
- Explicit size limit before parsing.
- Duplicate-key rejection.
- Unknown-field rejection by default.
- No floats in any value that reaches hashing or certified config material.
- Canonical JSON bytes for config and seed material.
- Authored input digest and canonical config digest.
- Redacted parse and validation errors.

REST rule:

- String config defaults to TOML.
- JSON object config implies JSON unless `config_format` is provided.
- If `config_format` is provided, it must match the value shape and selected op support.

Verification:

```sh
cargo test -p mfm-app authored_config
cargo test -p mfm authored_config
cargo test -p mfm-integration-tests --test rest_api_run_control authored_config
```

### Commit 4: Convert Portfolio Snapshot Into A Launchable Entry-Point Op

Goal: make `portfolio_snapshot` launchable through the registry.

Changes:

- Expose deterministic portfolio entry-point planning from the portfolio op crate.
- Keep authored portfolio config minimal and non-secret.
- Convert authored config into canonical portfolio config.
- Return `EntryPointOpPlan` with:
  - typed program draft
  - canonical config material
  - seed material if needed
  - public output schema id
  - lowering/canonicalizer identity
- Add registry construction for `portfolio_snapshot`.

Do not:

- Keep `mfm_cli portfolio snapshot`.
- Keep REST `/v1/portfolio/snapshot`.
- Connect stores from the portfolio op path.
- Render public output from in-memory op results.

Verification:

```sh
cargo test -p mfm-op-portfolio-tracker
cargo test -p mfm-app portfolio_snapshot
```

### Commit 5: Convert EVM Contract Lifecycle Entry Points

Goal: make EVM contract operations launchable through the same registry.

Entry-point op names:

- `evm_contract_deploy`
- `evm_contract_configure`
- `evm_contract_validate`
- `evm_contract_lifecycle`

Changes:

- Expose deterministic EVM entry-point planning from the EVM lifecycle op crate.
- Convert authored configs into canonical typed phase config.
- Preserve seed handling for configure/validate through `EntryPointOpPlan`.
- Return public output schema id.
- Keep signer refs and network ids non-secret.

Do not:

- Keep `mfm_cli evm contracts ...`.
- Keep REST `/v1/evm/contracts/*`.
- Read RPC/signing env vars from op planning.
- Move signer or transport resolution into op crates.

Verification:

```sh
cargo test -p mfm-op-evm-contract-lifecycle
cargo test -p mfm-app evm_contract
```

### Commit 6: Rework App Launch Assembly

Goal: make app assembly the single place that turns `EntryPointOpPlan` into a run launch.

Implement:

- Resolve public op name to latest version unless explicit version is supplied.
- Certify the typed program draft.
- Build launch material from `EntryPointOpPlan`.
- Prepare `RunLaunchRequest`.
- Record launch evidence:
  - submitted public op name
  - resolved typed op id/version
  - entry-point registry digest
  - lowering identity
  - canonicalizer identity
  - config format
  - authored config digest
  - canonical config digest
- Preserve existing run admission and scheduler semantics.

Remove:

- App helpers whose purpose is parsing or launching bundle-shaped transport JSON.
- Public structs/constants/error codes/rustdoc naming bundle-shaped launch.

Verification:

```sh
cargo test -p mfm-app
cargo test -p mfm-integration-tests --test architecture_namespace_contract
```

### Commit 7: Replace CLI `run start`

Goal: make CLI execution ingress op-based and delete old CLI launch surfaces.

Changes:

- `mfm_cli run start --op <NAME> --config <PATH>`
- `--op-version <VERSION>`
- `--config-format <json|toml>`, defaulting to TOML
- automatic public output rendering when completed and available
- existing `run resume`, `run status`, `run stream`, `run replay`, `run public-output`, and
  `run manual-resolution` stay as run-control/read surfaces

Delete:

- `bin/cli/src/commands/portfolio/*`
- `bin/cli/src/commands/evm/*` if no non-run command remains
- top-level `Portfolio` and `Evm` command variants when empty
- old CLI tests asserting portfolio/EVM command presence
- public bundle flag parsing and docs/tests

Do not add aliases.

Verification:

```sh
cargo test -p mfm --test cli_tests
cargo test -p mfm --test json_output_integration
cargo test -p mfm --test portfolio_snapshot_integration
```

The portfolio integration test should be rewritten or renamed to exercise:

```sh
mfm_cli run start --op portfolio_snapshot --config <PATH>
```

### Commit 8: Replace REST `/v1/runs/start`

Goal: make REST execution ingress op-based and delete old REST domain routes.

Changes:

- `/v1/runs/start` accepts entry-point op starts only.
- String config defaults to TOML.
- JSON object config implies JSON.
- Optional `op_version`.
- Automatic public output rendering when completed and available.

Delete:

- `/v1/portfolio/snapshot`
- `/v1/evm/contracts/deploy`
- `/v1/evm/contracts/configure`
- `/v1/evm/contracts/validate`
- `/v1/evm/contracts/lifecycle`
- route handlers, request structs, response structs, helpers, tests, and docs for those routes
- public bundle-shaped request parsing

Do not add compatibility routes.

Verification:

```sh
cargo test -p mfm-integration-tests --test rest_api_run_control
cargo test -p mfm-integration-tests --test parity_rest_api_postgres_typed_smoke
```

### Commit 9: Remove Bundle Terminology From Design And Public Docs

Goal: finish the bundle removal across documentation and public API surfaces.

Update:

- `docs/design.md`
- `docs/architecture.md`
- `bin/cli/README.md`
- `bin/rest-api/README.md`
- `crates/app/README.md`
- rustdoc for app launch APIs

Remove or rename:

- `CERTIFIED_SPEC_BUNDLE_KIND`
- `CertifiedSpecBundle` public transport usage if exposed by app-facing launch paths
- tests and fixtures with bundle-shaped request helpers
- error codes such as `CertifiedBundleInvalid` where they remain public

Keep:

- internal certified spec and certificate authority values where required by the runtime/certifier
  contract.

Verification:

```sh
rg -n "bundle|Bundle|CERTIFIED_SPEC_BUNDLE|CertifiedBundle" docs bin crates tests
cargo test -p mfm-integration-tests --test architecture_namespace_contract
```

The `rg` command should return no public/API-facing bundle terminology. If internal certifier terms
remain, document why they are not public transport concepts.

### Commit 10: End-To-End Portfolio Through `run start --op`

Goal: prove the new path works for a representative non-trivial workflow.

Tests:

- CLI starts `portfolio_snapshot` from TOML.
- REST starts `portfolio_snapshot` from TOML string.
- REST starts `portfolio_snapshot` from JSON object.
- Public output is included automatically when the run completes.
- `run public-output` still reads the stored public output by verified authority.
- Run evidence includes op name/version and config digests.

Verification:

```sh
cargo test -p mfm --test json_output_integration portfolio
cargo test -p mfm-integration-tests --test typed_portfolio_snapshot_local
cargo test -p mfm-integration-tests --test rest_api_run_control portfolio
```

### Commit 11: End-To-End EVM Contract Flows Through `run start --op`

Goal: prove the EVM flows work through the new generic execution ingress.

Tests:

- Deploy op starts through CLI/REST generic op path.
- Configure op starts through CLI/REST generic op path.
- Validate op starts through CLI/REST generic op path.
- Full lifecycle op starts through CLI/REST generic op path.
- Existing secret-redaction assertions still hold.
- Runtime env vars remain runtime-only and do not enter config artifacts, events, public output, or
  test snapshots.

Verification:

```sh
cargo test -p mfm-integration-tests --test parity_evm_contract_lifecycle_reth
cargo test -p mfm-integration-tests --test rest_api_run_control evm
```

Run live parity tests only with the required services and environment variables explicitly set.

### Commit 12: Final Deletion Sweep

Goal: remove leftovers and prove there is no old surface.

Run targeted scans:

```sh
rg -n "portfolio snapshot|PortfolioSnapshot|portfolio/snapshot" bin crates tests docs
rg -n "evm contracts|EvmContract.*Start|/v1/evm/contracts" bin crates tests docs
rg -n "bundle|Bundle|CERTIFIED_SPEC_BUNDLE|CertifiedBundle" bin crates tests docs
rg -n "run start --bundle|--bundle" bin crates tests docs
```

Delete anything that is not required by the new architecture.

Verification:

```sh
cargo fmt --all -- --check
cargo check --workspace
cargo test -p mfm --test cli_tests
cargo test -p mfm --test json_output_integration
cargo test -p mfm-integration-tests --test architecture_namespace_contract
cargo test -p mfm-integration-tests --test cargo_metadata_contract
```

Run broader workspace tests when feasible:

```sh
cargo test --workspace
```

## Review Checklist

Before merging the implementation series:

- CLI and REST are thin transport layers.
- No compatibility aliases or hidden fallbacks remain.
- Entry-point ops plan only; they do not certify, launch, read env vars, connect stores, or render.
- App assembly owns registry construction, certification, launch preparation, and run admission.
- Runtime executes only certified typed specs.
- Public output rendering uses verified read authority after persisted run evidence exists.
- Authored config normalization enforces the security/canonicalization invariants.
- Run evidence records op resolution and config digests.
- Old EVM/portfolio public execution surfaces are gone.
- Public bundle-shaped launch is gone.
- Documentation and tests describe only the new entry-point op path.
