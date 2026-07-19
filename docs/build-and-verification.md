# Rust build and verification

Status: contributor and operator contract.

MFM uses one Nix-pinned tool environment with different artifact policies for
development, verification, and release packaging. Cargo remains the sole
authority for the Rust dependency graph and mutable compiler artifacts.
Nixfied executes the verification graph and owns its processes, services,
endpoints, runtime state, and run evidence.

## Build lanes

| Lane | Entry points | Artifact location | Policy |
| --- | --- | --- | --- |
| Focused development | `nix develop`, direct Cargo, `nix run .#quick` | normal worktree `target` | mutable, incremental, developer-owned |
| Broad verification | `nix run .#check`, `.#test`, `.#test-db`, `.#ci` | worktree `target/verification` | mutable, compact, nonincremental |
| Release packaging | `nix build .#mfm` | Nix store | immutable package output |

These are assurance and artifact boundaries, not competing Cargo and Nix
environments. Development and verification both use the pinned Rust toolchain
and native dependencies supplied by Nix. Release packaging does not act as a
test cache, and mutable Cargo artifacts are never release inputs or trusted
verification results.

`nix run .#mfm` is a local runtime convenience around the immutable package:
it starts the Nixfied-managed PostgreSQL service, supplies `DATABASE_URL`, and
then delegates to the packaged CLI. It is not a separate build lane or a raw
package entry point.

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

Enter the pinned shell and use targeted Cargo commands while iterating:

```bash
nix develop
cargo check -p <package>
cargo test -p <package>
```

`nix develop` and `.#quick` explicitly leave `CARGO_TARGET_DIR` unset, so they
use Cargo's ordinary worktree target and retain incremental compilation.
`.#quick` runs formatting and workspace library/binary checking only; it is a
feedback loop, not a merge gate.

Tests that need live infrastructure may be run directly after starting the
required services manually and setting explicit variables such as
`DATABASE_URL` or `MFM_RUNTIME_CONFIG_FILE`.

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
cargo clean --target-dir target/verification
```

Deleting a worktree also deletes its verification target. Capacity remains a
project/worktree concern; Nixfied does not inventory, account for, retain, or
garbage-collect compiler artifacts.

## Gates

| Command | Contract |
| --- | --- |
| `nix run .#model-check` | Admit the compiled Nixfied model without running project tasks. |
| `nix run .#check` | Run formatting, Clippy, architecture/Cargo metadata contracts, and offline SQLx checking. |
| `nix run .#test` | Run workspace Nextest and doctests without managed external services. |
| `nix run .#test-db` | Start managed PostgreSQL, check online SQLx schema metadata, and run PostgreSQL parity tests. |
| `nix run .#ci` | Run the complete graph, including the component gates and feature-gated parity coverage. |

The definitions in `nixfied.nix` are authoritative when individual tests or
task counts evolve. Before a commit, run `.#check`, `.#test`, and `.#test-db`.
Run `.#ci` after major work and for final merge readiness.

Nixfied owns deterministic service endpoint placement. Starts from independent
state roots are coordinated by the upstream endpoint contract; an occupied
planned endpoint is reported as `PORT_CONFLICT`. This runtime responsibility
is independent of Cargo target placement.

## Evidence behind the policy

The local Linux measurements that selected this policy established:

- compact verification artifacts reduced the target from roughly 17–18 GiB
  to 9.8 GiB while eliminating incremental artifacts and `.dwo` files and
  retaining source-line diagnostics;
- a stable native Cargo target measured a 359.204-second clean median and a
  197.644-second warm median, saving 161.560 seconds or 45.0%;
- the stable target occupied approximately 10.49 GB per worktree; and
- the measured assurance inventory remained unchanged throughout the selected
  candidate's qualification runs.

Those timings were collected on one reference `aarch64-linux` host. They are
selection evidence, not a performance SLA or a macOS measurement. The same
portable mechanism is required on supported platforms, while local Linux is
the performance-measurement authority.

The alternatives were rejected independently:

| Candidate | Reason not selected |
| --- | --- |
| `sccache` | Empty-cache verification regressed 17.4%, and second-worktree reuse missed the materiality threshold. |
| Crane artifacts | Warm full-gate improvement was only 4.4%/9.05 seconds and still required mutable Trybuild state. |
| `crate2nix` | Per-crate derivations did not preserve the complete repository, Trybuild, doctest, and parity verification surface. |
| `cargo2nix` | The generated graph did not preserve Cargo dependency/feature fidelity or the complete test surface. |
| Execution-topology changes | Measured end-to-end savings missed both the 15% and 30-second materiality thresholds. |

No combination of rejected candidates is selected, and MFM does not maintain a
second per-crate Nix representation of Cargo's build graph.

## Changing this contract

Changes to target placement, compiler wrappers, per-crate derivations, hosted
persistence, or verification topology must be measured through the complete
consumer workload. At minimum, a proposal must demonstrate:

- unchanged current test identifiers, features, doctests, Trybuild cases,
  SQLx checks, parity behavior, and public gate composition;
- focused developer behavior remains incremental and independent of the broad
  verification target;
- correct invalidation for leaf, shared-crate, non-Rust, toolchain, and target
  configuration changes;
- nested Cargo path correctness, same-worktree concurrency, cancellation
  recovery, exact cleanup, and repeated-run storage growth;
- local Linux clean and warm distributions, with both relative and absolute
  materiality reported; and
- `nix run .#ci` passing on the final authored model.

Remote or hosted caches additionally require an explicit trust, credentials,
corruption, outage, retention, and cost contract. They must not be introduced
as an implicit extension of local Cargo reuse.

For coordinated Nixfied pin and runtime ABI changes, follow
[`UPGRADE.md`](UPGRADE.md).
