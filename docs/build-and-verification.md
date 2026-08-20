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
nix develop -c cargo test -p mfm-storage-postgres --lib
nix develop -c cargo test -p mfm-runtime --all-targets
nix develop -c cargo test -p mfm-evm -p mfm-portfolio -p mfm-evm-live --all-targets
nix develop -c cargo test -p mfm-app --all-targets
nix develop -c cargo test -p mfm --all-targets
nix develop -c cargo test -p mfm-rest-api --all-targets
```

The CLI e2e is ignored by default. Drive it directly only against a locally started pinned
`reth --dev` plus PostgreSQL, and only serially:

```bash
MFM_E2E_RPC_URL=... \
MFM_E2E_RUNTIME_STORE_LOCATOR=... MFM_E2E_ADMIN_STORE_LOCATOR=... nix develop -c cargo test \
  -p mfm --test cli_e2e -- --include-ignored --test-threads=1
```

## Nixfied tasks

| Command | Contract |
| --- | --- |
| `nix run .#model-check` | Admit the compiled model without project tasks. |
| `nix run .#run -- --task postgres-test` | Run private ignored PostgreSQL tests through real hostname-verified TLS, exact pinned roots, hostile ambient settings, and the split runtime role; wrong pin/CA/host are rejected. |
| `nix run .#run -- --task transport-authority-test` | Exercise the production EVM HTTPS client through exact pinned roots and prove proxy, redirect, wrong-pin, alternate-CA, and wrong-host authority are rejected. |
| `nix run .#run -- --task cli-e2e` | Exercise the complete stored-config CLI lifecycle against managed TLS EVM and split-authority TLS PostgreSQL fixtures. |
| `nix run .#run -- --task rest-e2e` | Build both client binaries explicitly and compare their shared JSON models through the production Unix-socket REST listener against managed TLS EVM/PostgreSQL. |
| `nix run .#run -- --task capacity-app` | Exercise the exact 64/65-source Portfolio Program/C0 bound. |
| `nix run .#run -- --task capacity-runtime` | Exercise hot/cold and zero-State Runtime progression. |
| `nix run .#run -- --task capacity-store` | Freeze Journal/Store object, frame, count, and cumulative-byte arithmetic. |
| `nix run .#run -- --task capacity-envelope` | Compose the three capacity owners above. |
| `nix run .#ci` | Compose format, Clippy, workspace check/tests, managed DB, EVM transport authority, both managed client e2es, docs, and capacity tasks. |

Do not run broad component gates immediately before `.#ci` on the same tree. Once focused failures
are resolved, run CI exactly once on the final candidate when the workflow requires the composed
gate. Report commands, results, and anything not run.

Nixfied verification uses `target/verification`, disables incremental compilation, caps Cargo jobs
at two, and reduces debug info. For a deliberate cold run, use:

```bash
nix develop -c cargo clean --target-dir target/verification
```

This removes only verification artifacts, not Nixfied state.
