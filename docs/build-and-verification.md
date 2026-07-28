# Rust build and verification

Status: contributor and operator contract. This document owns workflow and
verification-selection policy; `nixfied.nix` owns exact executable task
composition.

MFM uses one Nix-pinned tool environment with different artifact policies for
development, verification, and release packaging. Cargo remains the sole
authority for the Rust dependency graph and mutable compiler artifacts.
Nixfied executes the verification graph and owns its processes, services,
endpoints, runtime state, and run evidence.

## Build lanes

| Lane | Entry points | Artifact location | Policy |
| --- | --- | --- | --- |
| Focused development | Cargo inside the default `nix develop` shell | normal worktree `target` | mutable, incremental, developer-owned |
| Broad verification | `nix run .#check`, `.#test`, `.#test-db`, `.#ci` | worktree `target/verification` | mutable, compact, nonincremental |
| Release packaging | `nix build .#mfm` | Nix store | immutable package output |

These are assurance and artifact boundaries, not competing Cargo and Nix
environments. Development and verification both use the pinned Rust toolchain
and native dependencies supplied by Nix. Release packaging does not act as a
test cache, and mutable Cargo artifacts are never release inputs or trusted
verification results.

`nix run .#mfm` is a project CLI convenience, not a separate build lane. Its managed database
lifecycle and raw execution alternatives are owned by the [CLI reference](../bin/cli/README.md).

## Responsibility boundary

| Surface | Owner | Responsibility |
| --- | --- | --- |
| Toolchain and native packages | Nix | Pin and realize platform-specific tools and immutable closures. |
| Cargo target internals | Cargo | Dependency planning, fingerprints, writer locking, invalidation, rebuild decisions, and corruption recovery. |
| Verification target policy | MFM | Select its path and profile, inspect capacity, decide retention, and provide exact cleanup. |
| Verification execution | Nixfied | Execute every selected task and own containment, cancellation, services, endpoints, state, logs, and run evidence. |
| Hosted jobs | CI provider | Execute repository gates; no persistent hosted compiler cache is selected. |

Artifact presence never substitutes for task execution. Nixfied evidence says
which tasks ran and what they returned; it does not report Cargo cache
identities, hits, misses, or retention state.

## Focused development

All developer-invoked Cargo/Rust tools run through the default Nix development
shell. Nix owns the toolchain and native dependency pins; Cargo remains
authoritative for the Rust graph and artifacts. Do not rely on host-installed
Rust tooling.

A host Rust installation is not part of the supported workflow and may be
absent. Editors and automation that invoke Rust tools must inherit the default
development shell or use `nix develop -c`.

Enter the shell once and use targeted Cargo commands while iterating:

```bash
nix develop
cargo fmt --all -- --check
cargo check -p <package>
cargo test -p <package> <test-filter>
```

For a non-interactive one-off command, use `nix develop -c cargo ...`.
The development shell leaves `CARGO_TARGET_DIR` unset, so Cargo uses its
ordinary worktree target and retains incremental compilation.

Prefer a named test target or filter before a whole-package test, and prefer a
whole-package test before a workspace test. Expand to affected dependents when
a public crate contract changes. Do not duplicate broad verification in both
the development and verification targets without a change-specific reason.

For a one-off check that already exists as a Nixfied leaf, invoke that task
directly instead of its enclosing gate:

```bash
nix run .#run -- --task cargo-metadata-contract
nix run .#run -- --task parity-postgres-rest-api
nix run .#run -- --task parity-bitcoin-core
```

The task invocation starts only its declared service requirements. Task ids
come from `nixfied.nix`; the examples above run the metadata contract without a
service, the REST parity target with managed PostgreSQL, and the Bitcoin parity
target with managed Bitcoin Core. Direct task runs use the broad verification
target and retain Nixfied evidence.

Leaf task ids are focused internal entry points, not stable public verbs.
Confirm the current id in `nixfied.nix`. Selecting a leaf runs that leaf and its
declared service requirements; it does not inherit predecessor tasks that an
enclosing composite adds. Use a leaf only when its test is independently valid.

For repeated parity debugging, the incremental lane can be faster: start the
required service once, set explicit variables such as `DATABASE_URL` or
provider values referenced by an explicit `--runtime-config <PATH>`, and
repeatedly run the focused Cargo test. This caller-managed path does not
produce Nixfied service or run evidence.

## Selecting verification scope

Verification follows the affected surface and risk, not the number of commits.
There is no blanket requirement to run every broad gate before each commit.
While iterating, use the focused lane above; when the change is coherent, run
the smallest final gate set that covers it.

| Change surface | Final local verification |
| --- | --- |
| Prose, comments, or non-executable documentation | Check changed links, examples, and command claims, then run `git diff --check`. No Rust or service gate is required unless the documentation changes an executable/generated contract or makes claims that need validation against one. |
| Local behavior within one crate | Run rustfmt, a package-scoped check or Clippy invocation, and the affected package/test targets. Include dependent packages when a public contract changed. |
| Cargo manifest, workspace metadata, Cargo-enforced crate taxonomy, or dependency-boundary configuration | Run affected package checks/tests and `nix run .#run -- --task cargo-metadata-contract`. Add `.#check` when workspace resolution or all-feature lint coverage changed broadly. |
| Cross-crate public API, proc-macro output, shared kernel/runtime semantics, or multi-crate behavior | Run `nix run .#check` and `nix run .#test`, unless a final `.#ci` run will cover them. |
| PostgreSQL migration, SQLx metadata/query, store behavior, or DB-backed CLI/REST behavior | Run focused tests during development, then `nix run .#test-db`. Add other gates only for surfaces they cover. |
| Nixfied model or verification graph | Run `nix run .#model-check` early and exercise the changed task/gate. Run `nix run .#ci` once on the final revision; use component gates separately only for diagnosis. |
| Flake output, package/dev-shell definition, flake dependency pin, or hosted workflow | Run `nix flake check --no-build` for early evaluation, then exercise the affected output or invocation. Add `.#ci` when the toolchain, Nixfied runtime, gate execution, or cross-platform behavior changed. |
| Security-sensitive, persisted-contract, scheduler/recovery, cross-cutting, release, or explicit full local merge-readiness validation | Run targeted checks first, then `nix run .#ci` once on the final revision. |

When a change spans rows, combine only non-overlapping coverage. Editing an
architecture or workflow document does not by itself select the code or model
row with the same subject.

Low-risk isolated changes can close locally with targeted evidence; hosted CI
remains the full cross-platform backstop after push. Always report the commands
that ran and any broader gate that did not. If uncertainty about the blast
radius remains, choose the broader applicable row.

## Broad verification

Broad Nixfied Cargo leaves use this project-owned policy:

```text
CARGO_TARGET_DIR=target/verification
CARGO_INCREMENTAL=0
CARGO_PROFILE_DEV_DEBUG=1
CARGO_PROFILE_TEST_DEBUG=1
CARGO_PROFILE_DEV_SPLIT_DEBUGINFO=off
CARGO_PROFILE_TEST_SPLIT_DEBUGINFO=off
CARGO_BUILD_JOBS=2
```

The task wrapper makes the target path absolute from the admitted worktree
root before invoking Cargo. Nested Cargo processes, including Trybuild and the
crate-local SQLx check, therefore inherit the same target even after changing
directories.

All Nixfied slots in one worktree share this target and rely on Cargo's writer
locking. Separate worktrees isolate naturally by filesystem path. The
Trybuild groups in `.config/nextest.toml` also avoid competing nested Cargo
writers within one Nextest run.

`NIXFIED_STATE_DIR` selects only Nixfied runtime state and evidence. It does
not relocate or clean Cargo artifacts, and `nix run .#clean` leaves
`target/verification` untouched.

## Artifact lifecycle and trust

The verification target is same-user mutable local state. It has no selected
cross-user sharing, remote transfer, signing, hosted persistence, or binary
cache trust contract. There is also no non-destructive same-namespace bypass.

No routine cleanup is required. For a deliberate cold run or suspected Cargo
artifact corruption, delete exactly the verification target through Cargo:

```bash
nix develop -c cargo clean --target-dir target/verification
```

## Gates

| Command | Contract |
| --- | --- |
| `nix run .#model-check` | Admit the compiled Nixfied model without running project tasks. |
| `nix run .#check` | Run formatting, Clippy, architecture/Cargo metadata contracts, and offline SQLx checking. |
| `nix run .#test` | Run main-workspace Nextest and doctests, including the pinned OS-isolation prototype, plus the locked nested authority-prototype tests and Clippy, without managed external services. |
| `nix run .#test-db` | Start managed PostgreSQL, check online SQLx schema metadata, run PostgreSQL parity and recoverability executor/HA prototype tests, and run the fenced executor PostgreSQL qualification matrix. |
| `nix run .#ci` | Run the complete graph, including the component gates and feature-gated parity coverage. |

The definitions in `nixfied.nix` are authoritative when individual tests or
task counts evolve. `.#ci` composes `.#check`, `.#test`, and `.#test-db`, then
adds keystore, Reth, and Bitcoin Core parity coverage before the closing source
revision. Do not run the three component gates immediately before `.#ci` on the
same revision: that repeats their work in separate Nixfied runs. Run a component
independently when it is the smallest sufficient boundary gate or when isolating
a failure.

The `parity-bitcoin-core` leaf starts an isolated loopback-only regtest node from
the root flake's pinned `pkgs.bitcoind`. The current executable version is
asserted as Bitcoin Core 31.0, both at Nix evaluation and service startup, and
must remain at least 28. The task mines a deterministic fixture, then exercises
the public checked Bitcoin batch transport against the live node for chain/IBD
validation, one multi-descriptor scan, scan-height hash confirmation,
zero/nonzero balances, and redacted authentication failure. Its data directory,
port, process lifetime, and ephemeral cookie credential at one deterministic
managed path are Nixfied-owned; no credential value enters the compiled model,
arguments, or logs, and the task never uses a host `bitcoind`. The flake lock is
the version pin. The transport contract tracks the official
[JSON-RPC interface](https://github.com/bitcoin/bitcoin/blob/v31.0/doc/JSON-RPC-interface.md),
[`scantxoutset`](https://bitcoincore.org/en/doc/31.0.0/rpc/blockchain/scantxoutset/),
[`getblockchaininfo`](https://bitcoincore.org/en/doc/31.0.0/rpc/blockchain/getblockchaininfo/),
and [`getblockhash`](https://bitcoincore.org/en/doc/31.0.0/rpc/blockchain/getblockhash/)
contracts.
