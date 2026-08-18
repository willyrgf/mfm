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
nix develop -c cargo check -p mfm -p mfm-rest-api --all-targets
```

## Nixfied tasks

| Command | Contract |
| --- | --- |
| `nix run .#model-check` | Admit the compiled model without project tasks. |
| `nix run .#run -- --task postgres-test` | Run private ignored same-crate PostgreSQL tests against the managed database; missing service/URL is a failure. |
| `nix run .#run -- --task capacity-app` | Exercise the exact 64/65-source Portfolio Program/C0 bound. |
| `nix run .#run -- --task capacity-runtime` | Exercise hot/cold and zero-State Runtime progression. |
| `nix run .#run -- --task capacity-store` | Freeze Journal/Store object, frame, count, and cumulative-byte arithmetic. |
| `nix run .#run -- --task capacity-envelope` | Compose the three capacity owners above. |
| `nix run .#ci` | Compose format, Clippy, workspace check/tests, managed DB, docs, and capacity tasks. |

Do not run broad component gates immediately before `.#ci` on the same tree. Once focused failures
are resolved, run CI exactly once on the final candidate when the workflow requires the composed
gate. Report commands, results, and anything not run.

Nixfied verification uses `target/verification`, disables incremental compilation, caps Cargo jobs
at two, and reduces debug info. For a deliberate cold run, use:

```bash
nix develop -c cargo clean --target-dir target/verification
```

This removes only verification artifacts, not Nixfied state.
