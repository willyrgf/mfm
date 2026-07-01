# RFC runtime config implementation plan

This document is the approved execution plan for `RFC_RUNTIME_CONFIG.md`.
It is documentation only: no implementation work is included in this change.

Architect review status:

> APPROVED: the implementation plan fully represents RFC_RUNTIME_CONFIG.md.

## Planning constraints

- Treat `RFC_RUNTIME_CONFIG.md` as the implementation contract.
- Breaking changes are allowed and expected.
- Do not preserve compatibility shims for old EVM runtime JSON environment
  surfaces.
- Prefer deleting obsolete or misplaced code over wrapping it.
- Keep ownership boundaries intact:
  - runtime config parsing in `mfm-runtime-config`;
  - live EVM route/source resolution in transports;
  - workflow-specific requirement extraction in adapters;
  - app assembly wires stores, transports, and signers;
  - binaries only parse/pass paths;
  - replay, status, and public-output paths do not depend on live runtime config.
- Runtime config must stay out of persisted artifacts, events, public output,
  committed secret-bearing fixtures, and replay authority.

## Commit 1: `add runtime config parsing crate`

Purpose:

- Add `mfm-runtime-config` as the only parser, value-source resolver, and shape
  validator for live runtime configuration.
- Support TOML and JSON from the first implementation.
- Support direct, env, file, and file_env value-source resolution.
- Keep this crate free of live IO beyond reading the config and referenced
  indirection files.
- Keep workflow semantics, app assembly, transports, stores, and signers out of
  this crate.

Likely touched:

- `Cargo.toml`
- `crates/runtime-config/Cargo.toml`
- `crates/runtime-config/src/lib.rs`
- `crates/runtime-config/tests/*`
- `tests/integration/tests/cargo_metadata_contract.rs`

Old code/surfaces to delete:

- None in this commit. This commit introduces the new crate before wiring.

Public/breaking behavior changes:

- None is wired into binaries yet.
- The new crate rejects source-level runtime `expected_chain_id` from the start.

Tests to add/update:

- TOML/JSON parsing parity.
- Direct/env/file/file_env value-source resolution.
- Exactly-one endpoint source validation.
- At-most-one auth source validation.
- URL userinfo rejection.
- Route/source/signer reference validation.
- Same-id policy synthesis and rejection cases.
- Duplicate policy-source mapping rejection.
- Signer shape validation.
- Rejection of direct passwords, private keys, mnemonics, and signed material.
- Whole-family validation, including malformed unused entries in a needed family.
- Closed redaction for runtime config paths, indirection paths, env-resolved
  paths, RPC URLs, auth values, passwords, private keys, keystore paths, unlock
  files, and signed material.

Focused verification:

- `cargo test -p mfm-runtime-config`
- `cargo test -p mfm-integration-tests --test cargo_metadata_contract`

Documentation updates:

- Add rustdoc examples with redacted `runtime.toml` and `runtime.json`.
- Do not commit real secret-bearing runtime config fixtures.

## Commit 2: `split evidence-only run services from live drivers`

Purpose:

- Refactor app, CLI, and REST composition so evidence-only services do not build
  live runtime drivers.
- Make status, stream inspection, replay, public-output rendering, list/watch,
  health, and readiness independent of runtime config, live EVM transports, and
  signer providers.

Likely touched:

- `crates/app/src/lib.rs`
- `bin/cli/src/support/run_store.rs`
- `bin/cli/src/commands/run/status.rs`
- `bin/cli/src/commands/run/stream.rs`
- `bin/cli/src/commands/run/replay.rs`
- `bin/cli/src/commands/run/public_output.rs`
- `bin/cli/src/commands/run/list.rs`
- `bin/rest-api/src/lib.rs`

Old code/surfaces to delete:

- Read-only service construction through live `production_runner_registry` or
  equivalent live-driver assembly.

Public/breaking behavior changes:

- Read-only paths no longer observe malformed or missing live capability config.

Tests to add/update:

- CLI read-only paths succeed with no runtime config.
- REST read-only paths succeed with no runtime config.
- Replay service construction has no live config source.
- Public-output rendering uses only store/artifact data.
- Replay/status/public-output succeed after runtime config, env vars, and secret
  files are removed.

Focused verification:

- `cargo test -p mfm-app`
- `cargo test -p mfm`
- `cargo test -p mfm-integration-tests --test rest_api_run_control`

Documentation updates:

- Update `docs/design.md` for replay independence.
- Update `docs/architecture.md` for evidence-only assembly boundaries.
- Update `docs/persisted-public-surfaces.md` for runtime config exclusion.

## Commit 3: `add postgres store authority guard`

Purpose:

- Add explicit Postgres store authority validation during store-backed app
  construction.
- Keep this guard outside runtime config and outside run evidence.
- Validate connection, schema, migrations, catalog metadata, and trust-scope
  binding with closed redacted diagnostics.

Likely touched:

- `crates/storages/stream-store-postgres/src/schema.rs`
- `crates/storages/stream-store-postgres/src/run_store/mod.rs`
- `crates/storages/stream-store-postgres/src/lib.rs`
- `crates/storages/stream-store-postgres/src/run_store/tests.rs`
- `crates/app/src/lib.rs`

Old code/surfaces to delete:

- Implicit or scattered store authority assumptions.
- Any diagnostics that can expose database URLs or credentials.

Public/breaking behavior changes:

- Store setup failures become closed redacted store-authority errors.
- `DATABASE_URL` and `--database-url` remain the only store selectors.
- Store authority validation applies before all store-backed services are
  constructed, including live start/resume and evidence-only status, stream
  inspection, replay, public-output rendering, list/watch, and readiness paths.

Tests to add/update:

- Unavailable Postgres.
- Migration count/checksum mismatch.
- Missing or stale schema object.
- Invalid metadata/trust scope.
- First-boot trust-scope source remains store-owned.
- Database URL is never emitted in errors, events, artifacts, CLI output, or REST
  output.

Focused verification:

- `cargo test -p mfm-stream-store-postgres --features parity-tests`
- `cargo test -p mfm-app`
- `nix run .#test-db`

Documentation updates:

- Update `crates/storages/stream-store-postgres/README.md`.
- Update `docs/design.md`.
- Update `docs/persisted-public-surfaces.md`.

## Commit 4: `cut over evm live runtime config and guarded requests`

Purpose:

- Perform one buildable vertical API break across capability contracts,
  transport, adapters, app assembly, binaries, tests, docs, and Nixfied.
- Avoid an intermediate commit where dependents fail to compile or old
  compatibility shims remain.
- App assembly maps typed `mfm-runtime-config` EVM and signer descriptors into
  transport and signer registries.
- EVM transports own live route/source resolution.
- Adapters derive guards from workflow semantics and perform defensive evidence
  verification.
- Binaries only parse and pass runtime config paths.

Likely touched:

- `crates/evm-capabilities/src/lib.rs`
- `crates/transports/evm/src/lib.rs`
- `crates/transports/evm/src/tests.rs`
- `crates/transports/evm/Cargo.toml`
- `crates/adapters/portfolio/src/lib.rs`
- `crates/adapters/portfolio/src/tests.rs`
- `crates/adapters/evm-contracts/src/lib.rs`
- `crates/adapters/evm-contracts/src/tests.rs`
- `crates/states/evm-contracts/src/lib.rs`
- `crates/states/evm-contracts/src/tests.rs`
- `crates/ops/evm-contract-lifecycle-op/src/lib.rs`
- `crates/app/src/lib.rs`
- `crates/app/src/evm_contracts.rs`
- `crates/app/src/evm_runtime_routes.rs`
- `crates/signers/keystore/src/lib.rs`
- CLI run command surfaces
- REST run command surfaces
- Integration tests
- `nixfied.nix`
- `AGENTS.md`
- `bin/cli/README.md`
- `bin/rest-api/README.md`
- `docs/evm-rpc-routing.md`
- `docs/design.md`
- `docs/architecture.md`
- `docs/persisted-public-surfaces.md`

Old code/surfaces to delete:

- `MFM_EVM_RPC_SOURCES_JSON`
- `MFM_EVM_NETWORK_ROUTES_JSON`
- `MFM_EVM_SIGNERS_JSON`
- `--evm-rpc-sources`
- Related old EVM compatibility inputs.
- `EvmJsonRpcClient::from_env`
- Legacy EVM source JSON parser.
- Source-level runtime `expected_chain_id`.
- `crates/app/src/evm_runtime_routes.rs` if it only exists for adapter-local
  route plumbing.
- App/env signer parsing.
- `KeystoreSignerRegistryEntry::from_env_sources`.
- Adapter-facing route wrappers and request fields exposing `source_ref` and
  `policy_id`.
- Mandatory operation-planned baseline EVM chain identity validation nodes or
  preludes where guarded provider calls replace them.

Public/breaking behavior changes:

- Live EVM CLI start/resume uses `--runtime-config <PATH>` or
  `MFM_RUNTIME_CONFIG_FILE`.
- REST accepts the same runtime config path source but must not fail startup when
  the file is absent or malformed.
- Runtime config validation errors are reported at live run start/resume request
  time.
- Missing route or signer bindings fail pre-admission for live EVM runs that
  require them.
- Read-only EVM workflows do not require signer bindings.
- Mutation workflows require signer bindings.
- Runtime config whole-family validation is enforced for families required by the
  certified run.

Guard evidence rules:

- Guard evidence may include redacted selected source/policy ids as audit
  provenance.
- Guard evidence also includes semantic `network_id`, expected chain id, and
  observed chain id when available.
- Public outputs and replay authority must not depend on or resolve selected
  source/policy ids against current runtime config.

Tests to add/update:

- All EVM requests carry `EvmChainGuard`.
- Transport receives explicit typed source/route registries from app assembly.
- `mfm-transports-evm` has no dependency on `mfm-runtime-config`.
- Deployment ingress performs no live IO.
- Whole-family runtime config validation blocks live runs when a needed family
  has an unused malformed entry.
- Non-EVM portfolio starts without EVM runtime config.
- Contract validation workflows do not require signer binding.
- Mutation workflows require signer binding without pre-admission unlock.
- CLI/REST errors redact runtime config path, RPC URL, auth, paths, keystore
  paths, unlock files, private keys, passwords, and signed material.
- Old EVM env vars and old flags are rejected or ignored according to the new
  documented contract, with no compatibility shim.
- Tests generate runtime config files in temp dirs only.

Focused verification:

- `cargo check --workspace`
- Targeted EVM capability, transport, adapter, app, CLI, REST, and integration
  tests.
- `cargo test -p mfm-integration-tests --test architecture_namespace_contract`

Documentation updates:

- Update CLI and REST docs in the same commit as the behavior change.
- Rewrite old EVM routing docs to describe runtime config and guarded calls.
- Update `docs/design.md` and `docs/architecture.md`.
- Update `docs/persisted-public-surfaces.md`.
- Update stale `AGENTS.md` guidance that names old EVM runtime JSON env vars.
- Update Nixfied documentation/examples.

## Commit 5: `harden evm guard evidence and replay verification`

Purpose:

- Finish guarded-call semantics.
- Add closed redacted chain-mismatch evidence.
- Add replay-side guard verification that uses recorded evidence only.
- Add request-specific identity checks across live EVM methods.

Likely touched:

- `crates/evm-capabilities/src/lib.rs`
- `crates/transports/evm/src/lib.rs`
- `crates/adapters/portfolio/src/lib.rs`
- `crates/adapters/evm-contracts/src/lib.rs`
- `crates/kernel/runtime/src/attempt.rs` if diagnostic category support needs
  extension
- Replay tests and integration tests

Old code/surfaces to delete:

- Ad hoc chain mismatch or provider diagnostic strings that could leak provider
  bodies, URLs, auth, request bodies, or signed material.

Public/breaking behavior changes:

- Chain mismatch is a post-`RunAdmitted` attempt/capability failure with closed
  evidence, not a deployment ingress failure.

Tests to add/update:

- Live chain mismatch appends `RunAdmitted` first, then redacted attempt failure
  evidence.
- Failure evidence includes only semantic network id, expected chain id, observed
  chain id when available, selected source ref, and policy id.
- Diagnostics exclude provider bodies, RPC URLs, auth, paths, request secrets,
  signed payloads, and raw transactions.
- Replay verifies recorded guard evidence without runtime config, env vars, or
  secret files.
- Block, log, receipt, and submit paths all enforce identity checks.
- Mutation path checks guard before nonce, fee, gas, sign, and submit.

Focused verification:

- Targeted EVM capability tests.
- Targeted EVM transport tests.
- Targeted adapter tests.
- Targeted integration replay tests.

Documentation updates:

- Update `docs/design.md` for guard-failure evidence.
- Update `docs/persisted-public-surfaces.md` for replay authority and public
  output exclusions.

## Commit 6: `add runtime config migration regression gates`

Purpose:

- Add final regression coverage and source scans preventing legacy runtime wiring
  from returning.
- Migrate docs, generated test examples, and Nixfied examples to
  `runtime.toml`/`runtime.json`.
- Keep committed fixtures placeholder-only and non-secret.

Likely touched:

- `tests/integration/tests/architecture_namespace_contract.rs`
- `tests/integration/tests/cargo_metadata_contract.rs`
- `tests/integration/tests/rest_api_run_control.rs`
- `tests/integration/tests/portfolio_snapshot_local.rs`
- `tests/integration/tests/parity_*.rs`
- `tests/integration/src/test_support.rs`
- `nixfied.nix`

Old code/surfaces to delete:

- Remaining old EVM env test helpers.
- Remaining old EVM runtime JSON docs outside historical RFC text.
- Remaining fixture references to old EVM runtime env surfaces.

Public/breaking behavior changes:

- None. This is a guardrail and regression-test commit.

Tests to add/update:

- Replay, status, stream inspection, and public-output rendering succeed after
  runtime config path/file, RPC env vars, signer env vars, referenced secret/value
  files, keystore paths, and unlock files are removed.
- Malformed runtime config does not block REST startup or read-only services.
- Live start/resume returns redacted deployment errors only when the certified run
  needs the malformed or missing family.
- Namespace scan forbids old env constants, old from-env constructors,
  source-level runtime `expected_chain_id`, adapter-facing `source_ref` and
  `policy_id`, and `--evm-rpc-sources`.
- Cargo metadata forbids `mfm-transports-evm` depending on
  `mfm-runtime-config`.
- Cargo metadata or namespace tests forbid ops/states depending on runtime config.
- Nixfied writes generated runtime config under the state dir with restrictive
  permissions and atomic rename where supported.

Focused verification:

- `cargo test -p mfm-integration-tests --test architecture_namespace_contract`
- `cargo test -p mfm-integration-tests --test cargo_metadata_contract`
- Focused REST control tests.
- `nix run .#model-check`
- `nix run .#check`
- `nix run .#test`
- `nix run .#test-db`
- `nix run .#ci`

Documentation updates:

- No planned standalone doc work unless scans identify stale public text.
- Any stale doc removals happen in this same commit.

## RFC traceability matrix

| RFC section / migration item / decision | Planned commits |
| --- | --- |
| Typed runtime config parsing, value resolution, and validation | 1 |
| TOML and JSON support | 1 |
| Direct/env/file/file_env value sources | 1 |
| Redacted closed-category diagnostics | 1, 3, 4, 5, 6 |
| Runtime config shape validation | 1, 4, 6 |
| Reject source-level runtime `expected_chain_id` | 1, 4, 6 |
| Evidence-only app assembly independent of live runtime config | 2, 6 |
| Replay/status/public-output independence | 2, 5, 6 |
| CLI `--runtime-config` for live run start/resume | 4, 6 |
| REST runtime config support without startup failure | 4, 6 |
| Store surface remains `DATABASE_URL` / `--database-url` | 3 |
| Postgres store authority guard | 3 |
| Deployment ingress validation for required networks/signers | 4, 6 |
| Ingress checks are binding-only and pre-admission with no live IO | 4, 6 |
| Signer bindings required only for mutation workflows | 4, 6 |
| Guarded EVM capability request/response contracts | 4, 5 |
| EVM transport route/source resolution | 4, 5 |
| EVM transport chain guard before every live operation | 4, 5 |
| Adapter guard derivation | 4, 5 |
| Defensive evidence verification by adapters/replay | 4, 5 |
| Signer effect-scoped validation | 4, 6 |
| Nixfied runtime config generation and passing | 4, 6 |
| Ingress vs guarded capability vs semantic validation tests | 4, 5, 6 |
| Replay after runtime config/env/secret files are removed | 2, 5, 6 |
| Docs and fixtures migration to `runtime.toml` / `runtime.json` | 1, 4, 6 |
| Remove old EVM routing docs/fixtures | 4, 6 |

## RFC migration item traceability

1. Typed runtime config parsing/value resolution/validation: commit 1.
2. Evidence-only services independent from live config: commits 2 and 6.
3. CLI `--runtime-config` start/resume: commit 4.
4. REST runtime config without startup failure: commits 4 and 6.
5. Store surface and store authority guard: commit 3.
6. Guarded EVM request/response contracts: commits 4 and 5.
7. Transport route resolution and pre-operation chain guard: commits 4 and 5.
8. Adapter guard derivation and defensive evidence verification: commits 4 and 5.
9. Postgres authority guard: commit 3.
10. Remove source-level runtime `expected_chain_id`: commits 1, 4, and 6.
11. Remove adapter route wrappers: commits 4 and 6.
12. Remove `--evm-rpc-sources` and EVM env compatibility inputs: commits 4 and 6.
13. Delete old env constants, from-env wiring, source-chain plumbing, and request
    route plumbing: commits 4 and 6.
14. Nixfied runtime config generation/passing: commits 4 and 6.
15. Ingress vs guarded capability vs semantic validation tests: commits 4, 5,
    and 6.
16. REST/read-only independence tests: commits 2, 4, and 6.
17. Store authority tests: commit 3.
18. Docs/tests migration to runtime config: commits 4 and 6.
19. Remove/rewrite old EVM routing docs/fixtures: commits 4 and 6.

## RFC decision traceability

| RFC decision | Planned commits |
| --- | --- |
| REST starts even when live runtime config is missing or malformed | 4, 6 |
| Live start/resume validates required bindings; evidence-only paths do not | 2, 4, 6 |
| Parsing/resolution/shape validation live only in `mfm-runtime-config` | 1, 4, 6 |
| No legacy JSON envs, `--evm-rpc-sources`, or source-level chain id | 4, 6 |
| Env names are selectors; resolved values/paths/secrets are redacted | 1, 4, 6 |
| TOML and JSON are both supported from the first implementation | 1 |
| Direct keystore/unlock paths are allowed but never persisted or public | 1, 4, 6 |
| EVM chain identity is a mandatory guarded capability invariant | 4, 5 |
| Postgres guard is sibling store authority, not runtime config or replay evidence | 3 |

## Validation model traceability

| Validation layer | Planned commits |
| --- | --- |
| Runtime config shape validation | 1, 4, 6 |
| Deployment ingress validation | 4, 6 |
| Guarded capability calls | 4, 5 |
| Semantic validation states preserved | 4, 5 |
| Evidence and replay verification | 2, 5, 6 |
| Architecture placement and dependency boundaries | 1, 4, 6 |

## Deletion checklist

- [ ] `MFM_EVM_RPC_SOURCES_JSON`
- [ ] `MFM_EVM_NETWORK_ROUTES_JSON`
- [ ] `MFM_EVM_SIGNERS_JSON`
- [ ] `--evm-rpc-sources`
- [ ] Related old EVM compatibility inputs.
- [ ] `EvmJsonRpcClient::from_env`.
- [ ] Old live runtime `from_env` constructors.
- [ ] Legacy EVM runtime source JSON parser.
- [ ] Source-level `expected_chain_id` in runtime config.
- [ ] App-local EVM route plumbing that exposes `source_ref` and `policy_id`.
- [ ] Adapter-local route wrappers that expose `source_ref` and `policy_id` to
  states/adapters.
- [ ] EVM capability request fields carrying adapter-selected `source_ref` and
  `policy_id`.
- [ ] App/env signer parsing.
- [ ] `KeystoreSignerRegistryEntry::from_env_sources`.
- [ ] Mandatory operation-planned EVM chain identity validation nodes or preludes
  where guarded capability calls replace them.
- [ ] Stale Nixfied generation of old EVM JSON env surfaces.
- [ ] Stale docs, test helpers, fixtures, and architecture allowlists for old EVM
  runtime JSON surfaces.

## Verification plan

Focused checks during development:

- `cargo test -p mfm-runtime-config`
- `cargo test -p mfm-app`
- `cargo test -p mfm`
- `cargo test -p mfm-stream-store-postgres --features parity-tests`
- Targeted EVM capability, transport, adapter, app, CLI, REST, replay, and
  integration tests after commits 4 and 5.
- `cargo test -p mfm-integration-tests --test cargo_metadata_contract`
- `cargo test -p mfm-integration-tests --test architecture_namespace_contract`
- Focused REST run-control tests.

Workspace checks:

- `cargo fmt --all -- --check`
- `cargo check --workspace`
- Targeted integration tests for ingress, guarded capability failure, semantic
  validation ordering, REST startup behavior, and replay independence.

Nix-managed gates:

- `nix run .#model-check`
- `nix run .#check`
- `nix run .#test`
- `nix run .#test-db`
- `nix run .#ci` for final merge-readiness validation.

## Sequencing notes and risks

- Commit 4 is intentionally larger than ideal because the EVM request API break
  crosses capabilities, transport, adapters, app assembly, binaries, tests,
  Nixfied, and docs. Splitting it further would either fail to build or require
  temporary compatibility shims that the RFC forbids.
- `mfm-runtime-config` should have its own cargo metadata category. It should
  not be folded into domain config.
- `mfm-transports-evm` must not depend on `mfm-runtime-config`; app assembly maps
  runtime config descriptors into transport-owned registries.
- Ops and states must not depend on runtime config.
- Runtime config test files should be generated in temp directories. Committed
  examples may show redacted placeholder configs only.
- Semantic workflow `expected_chain_id` remains valid typed workflow config.
  Only runtime source-level `expected_chain_id` is removed and rejected.
