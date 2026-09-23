# Rust build and verification

This document owns verification selection; `nixfied.nix` owns exact task composition. All direct
Cargo/Rust commands run inside the default Nix development shell.

## Lanes

| Lane | Command | Artifacts |
| --- | --- | --- |
| Focused development | `nix develop -c cargo ...` | mutable worktree `target/` |
| Broad verification | `nix run .#ci` | `target/verification`, nonincremental |
| Release packaging | `nix build .#mfm` | immutable Nix output |

Cargo owns dependency planning and target locking. Nix pins tools/native dependencies. Nixfied owns
task execution, services, endpoints, state, and evidence. Artifact presence never substitutes for a
task result.

`nix/rust-toolchain.nix` owns the Rust package and executable environment shared by the default
development shell, every managed Cargo task, and release packaging. It pins Cargo, rustc, rustdoc
and rustfmt and disables inherited compiler wrappers. Clippy selects its sibling driver from the
same package. The shell preserves the caller's Cargo home and caches.

Invoke external Cargo tools directly inside the shell: `cargo-clippy clippy`, `cargo-fmt fmt`, and
`cargo-sqlx sqlx`. Cargo's `cargo clippy`, `cargo fmt`, and `cargo sqlx` spellings search
`CARGO_HOME/bin` before `PATH` and can select host-installed tools even inside Nix. Those spellings
are outside the pinned verification contract. Ordinary built-in commands such as `cargo check`,
`cargo test`, and `cargo build` use the shared pin.

Use the narrowest focused command while iterating:

```bash
nix develop -c cargo check -p <package> --all-targets
nix develop -c cargo test -p <package> <filter> -- --nocapture
nix develop -c cargo-fmt fmt --all -- --check
nix develop -c cargo-clippy clippy --workspace --all-targets --all-features -- -D warnings
```

Expand to affected dependents for a public contract change. A docs-only change needs link/command
review and `git diff --check`; it does not automatically select Rust gates. Cross-crate APIs,
persistence, concurrency, manifests, or task-graph changes require affected focused tests and one
final CI run. Run `nix run .#model-check` early after a Nixfied edit and
`nix flake check --no-build` after flake/output edits.

## Current focused matrix

```bash
nix develop -c cargo test \
  -p mfm-ids -p mfm-values -p mfm-program-derive -p mfm-capabilities --all-targets
nix develop -c cargo test -p mfm-program -p mfm-journal --all-targets
nix develop -c cargo test -p mfm-store --all-targets
nix develop -c cargo test -p mfm-storage-postgres --lib
nix develop -c cargo test -p mfm-runtime --all-targets
nix develop -c cargo test -p mfm-chain --all-targets
nix develop -c cargo test -p mfm-evm -p mfm-portfolio -p mfm-evm-live --all-targets
nix develop -c cargo test -p mfm-app --all-targets
nix develop -c cargo test -p mfm --all-targets
nix develop -c cargo test -p mfm-rest-api --all-targets
```

The cross-transport client execution e2e is ignored by default because it requires both built
binaries, a fresh split-role PostgreSQL schema, and pinned Reth. Drive it serially through the
managed `client-e2e` task below; that task owns the complete fixture and binary setup.

The contract-Effect recovery e2e is also ignored by default. Its managed task provisions the base
run/configuration surfaces and the separate optional transaction-authority surface, plus pinned Reth
and exact `solc 0.8.33`. The task compiles the first-party fixture to a temporary initcode file and
drives the ignored `mfm-evm-live` test serially; compiler output is never committed.

The Effect e2e reconstructs Runtime and database handles while retaining the same keystore owner;
its cold-recovery claim is not a process-restart test. Test supervision and funding/recovery helper
timeouts are fixture policy, not Runtime deadlines. Ordinary terminal progression uses Runtime and
native Pending pacing. Terminal checks prove unchanged history, output and nonce; a no-provider-IO
claim additionally needs explicit instrumentation. Focused native transaction tests retain exact
prepared-wire, rejecting-signer, cancellation and ambiguous-append coverage.

## Scalar-contract artifact

The `effect-e2e` task in `nixfied.nix` supplies pinned solc 0.8.33, compiles the first-party source with
no optimizer and `--evm-version cancun`, and sets `MFM_EFFECT_E2E_INITCODE_PATH` for the consuming
recipe/lifecycle tests. Run the managed task for complete reproduction:

```sh
nix run .#run -- --task effect-e2e
```

Its compiler recipe, with `solc` supplied by that pinned task environment, is:

```sh
fixture_dir="$(mktemp -d)"
solc --bin --overwrite --evm-version cancun --output-dir "$fixture_dir" \
  crates/live/evm/tests/fixtures/MfmEffectFixture.sol
export MFM_EFFECT_E2E_INITCODE_PATH="$fixture_dir/MfmEffectFixture.bin"
```

The decoded initcode is 497 bytes with SHA-256
`c7aed441e0afa86de84779ac168d27e4d8a29148b6834565d5930ef7c8ae855d`. The native artifact contract
checks this exact supported ABI/compiler output. Compiler output is temporary and is not committed;
the task removes its temporary directory. A different artifact requires a reviewed contract change.

## Acceptance scenarios and independent oracles

Each row names a consuming boundary, not a separate scenario DSL. Expected values come from fixture
premises or independent native observations, not only the production result projection being tested.
Focused owners retain guarantees that do not require repeating managed IO.

| Scenario | Independent oracle and retained boundaries | Executable owner |
| --- | --- | --- |
| Select Portfolio through CLI/REST | Independent runs preserve explicit/generated RunIds, exact selected revision and typed output; fixture balances and decimal amounts are checked against literal expectations and an independent RPC anchor/balance observation. Terminal cold inspection preserves head/output. | `bin/rest-api/tests/client_execution_e2e.rs`, managed `client-e2e` |
| Recover selection and publish enrichment | Delete admitted source configuration, cold-resume the same run, preserve provider error causes, publish retained enrichment without live discovery, repeat publication idempotently, and recover dependent selection after revision deletion. | Same managed client test and Application use-case tests |
| Compose lifecycle 42/84 | Maintained lifecycle yields 42; production Pure addition constructs later calldata 84 from 42+42. Assert deployment/configuration target, receipt point, exact native outcomes and independent node code/value observations. Pure addition consumes no nonce. | `crates/live/evm/tests/evm_contract_effect_e2e.rs`, managed `effect-e2e`; domain `lifecycle_runtime` |
| Preserve transaction authority | Reservation acknowledgement loss causes no early signing/submission; cancellation after broadcast retains exact command; cold recovery does not re-sign. External nonce advance affects only fresh transactions. SQL/closed-signer originals and local epoch rejection preserve their distinct contracts. The existing standalone deployment completes after authority restoration on the same RunId, retaining its exact command and failure-history prefix. Terminal replay preserves head/output/nonce. | Same managed lifecycle test and Live `transaction_tests.rs` |
| Extend with a new State | A consuming crate owns new semantic contracts and composes them with existing components; checked success and exact typed rejection survive cold inspection. Incompatible adjacency is rejected at compile time. | Domain `lifecycle_runtime` and Program compile-fail/authoring tests; current public examples in the authoring guide |
| Native qualification and unsuccessful outcomes | Same-ledger wrong route causes no provider call/outcome append; rejected/safe/integrity evidence cannot become a successful report. Native decoder/encoder phases and originals survive exact cold inspection. | EVM/Chain contracts, Live Portfolio client tests and Runtime callback tests |
| Runtime/Store safety | Manual exact-candidate reconciliation yields while automatic execution may continue from the checked retained command. Original acknowledgement precedes classification; cancellation preserves authority; competing/ambiguous appends do not invent acknowledgement; checkpoint barriers, report/frame capacity and cold ABI mismatches retain their exact failures. | Runtime/Program boundary suites, Journal/Store and managed `postgres-test` |

The managed extension policy proposed in earlier design discussions (42+8, ceiling 49 rejection) is
[deferred](known-gaps.md#deferred-product-execution-and-extension-scenarios); the generic extension
row does not claim that business oracle has been implemented. No Runtime wait-budget scenario is
required: deadlines were excluded from the current API. External test supervision remains separate.

A test/helper rewrite must map every removed observable assertion to its retained owner; historical
passes or proposed future scenarios do not prove a changed candidate.

## Checked PostgreSQL SQL

All static production SQL in `mfm-storage-postgres` uses SQLx 0.9 macros and the root `.sqlx`
metadata. `.cargo/config.toml`, verification tasks, and release packaging default to
`SQLX_OFFLINE=true`; ordinary compilation needs no database. After changing a query or a baseline:

```bash
nix run .#run -- --task sqlx-prepare
nix run .#run -- --task sqlx-check
```

Both tasks use the pinned PostgreSQL 18 service and SQLx CLI. They create a uniquely named disposable
database, replay the three existing baseline SQL files in order, and drop only that database on
exit. They create the baseline grantee role if absent without changing an existing role. The
passwordless managed admin endpoint is used only during Describe; no production locator, secret,
or runtime provisioner is needed. `--no-dotenv` prevents dotenv discovery. Prepare can bootstrap
an absent cache and updates `.sqlx`; review and commit its JSON with the query changes. Check is
read-only with respect to tracked content and rejects extra entries as well as stale/missing ones.
CI runs check before Rust compilation. A Nix build includes newly added cache/config files only
after they are staged in Git.

## Nixfied tasks

| Command | Contract |
| --- | --- |
| `nix run .#run -- --task sqlx-prepare` | Regenerate checked-query metadata from a disposable baseline database. |
| `nix run .#run -- --task sqlx-check` | Verify metadata content and the exact query filename set without updating tracked files. |
| `nix run .#model-check` | Admit the compiled model without project tasks. |
| `nix run .#run -- --task reth-smoke` | Co-start instant and delayed Reth with independently modeled loopback RPC and peer listeners; run the protocol smoke probe. |
| `nix run .#run -- --task postgres-test` | Run private ignored PostgreSQL tests through a real loopback-only `hostnossl` server, hostile overwritten ambient settings, isolated `PGOPTIONS` rejection, and the split runtime role. |
| `nix run .#run -- --task client-e2e` | Generate and interrupt an exact historical REST run at its first live Read, prove the durable runnable prefix, delete its config, cold-resume it against Reth, validate and reload its exact snapshot through the CLI, then reimport the same revision and require an independent CLI-generated run to produce the same semantic result. Also preserve one supplied operational provider error through cold REST and CLI observations, and run candidate enrichment through REST, delete its config, publish via REST, repeat publication through CLI, execute the dependent snapshot and recover its exact start after deleting the published revision. |
| `nix run .#run -- --task effect-e2e` | Run maintained scalar recipes and lifecycle tests with pinned solc, then the PostgreSQL/keystore lifecycle on ten-second interval-mining Reth. Lose the first reservation acknowledgement, cancel after actual broadcast and cold-recover exact retained commands without re-signing. Check lifecycle 42, an existing-address call after external nonce advance, and composed lifecycle 84 (`0 -> 2 -> 3 -> 4 -> 6`). Assert real absent receipts, known transactions and five unique native submissions. Preserve SQL causes, local epoch rejection without append, closed-signer custody and exact terminal cold replay. |
| `nix run .#run -- --task capacity-app` | Run the EVM, Portfolio and App test suites. |
| `nix run .#run -- --task capacity-runtime` | Exercise hot/cold and zero-State Runtime progression. |
| `nix run .#run -- --task capacity-store` | Freeze Journal/Store object, frame, count, and cumulative-byte arithmetic. |
| `nix run .#run -- --task capacity-envelope` | Compose the three capacity owners above. |
| `nix run .#ci` | Compose format, Clippy, workspace check/tests (including capacity coverage), managed DB, the managed client and Effect e2es, and docs. |

The standalone capacity tasks select tests already included in the workspace test stage. CI runs
that coverage once through `cargo-test`; it does not invoke `capacity-envelope` again. Keep these
commands for focused capacity verification. Follow the [test value policy](code-quality.md#test-value)
and acceptance ownership table above before changing coverage.

Do not run broad component gates immediately before `.#ci` on the same tree. Once focused failures
are resolved, run CI exactly once on the final candidate when the workflow requires the composed
gate. Report commands, results, and anything not run.

Nixfied verification uses `target/verification`, disables incremental compilation, caps Cargo jobs
at two, and reduces debug info. For a deliberate cold run, use:

```bash
nix develop -c cargo clean --target-dir target/verification
```

This removes only verification artifacts, not Nixfied state.
