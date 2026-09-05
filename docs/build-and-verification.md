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

Use the narrowest focused command while iterating:

```bash
nix develop -c cargo check -p <package> --all-targets
nix develop -c cargo test -p <package> <filter> -- --nocapture
nix develop -c cargo fmt --all -- --check
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
nix develop -c cargo test -p mfm-evm-transaction-authority --all-targets
nix develop -c cargo test -p mfm-storage-postgres --lib
nix develop -c cargo test -p mfm-runtime --all-targets
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
| `nix run .#run -- --task postgres-test` | Run private ignored PostgreSQL tests through a real loopback-only `hostnossl` server, hostile overwritten ambient settings, isolated `PGOPTIONS` rejection, and the split runtime role. |
| `nix run .#run -- --task client-e2e` | Generate and interrupt an exact historical REST run at its first live Read, prove the durable runnable prefix, delete its config, cold-resume it against Reth, validate and reload its exact snapshot through the CLI, then reimport the same revision and require an independent CLI-generated run to produce the same semantic result. |
| `nix run .#run -- --task effect-e2e` | Generate and fund an ephemeral keystore wallet, lose the first committed reservation acknowledgement before broadcast, cold-recover deployment and configuration through two durable Effects, decode the anchored getter as 42, prove pending nonce `0 -> 2`, and prove a fresh cold read/resume changes neither the terminal head/value nor the nonce. |
| `nix run .#run -- --task capacity-app` | Exercise the exact 64/65-source Portfolio Program/C0 bound. |
| `nix run .#run -- --task capacity-runtime` | Exercise hot/cold and zero-State Runtime progression. |
| `nix run .#run -- --task capacity-store` | Freeze Journal/Store object, frame, count, and cumulative-byte arithmetic. |
| `nix run .#run -- --task capacity-envelope` | Compose the three capacity owners above. |
| `nix run .#ci` | Compose format, Clippy, workspace check/tests, managed DB, the managed client and Effect e2es, docs, and capacity tasks. |

Do not run broad component gates immediately before `.#ci` on the same tree. Once focused failures
are resolved, run CI exactly once on the final candidate when the workflow requires the composed
gate. Report commands, results, and anything not run.

Nixfied verification uses `target/verification`, disables incremental compilation, caps Cargo jobs
at two, and reduces debug info. For a deliberate cold run, use:

```bash
nix develop -c cargo clean --target-dir target/verification
```

This removes only verification artifacts, not Nixfied state.
