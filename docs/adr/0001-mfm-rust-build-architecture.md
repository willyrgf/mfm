# ADR 0001: MFM Rust build architecture

- Status: accepted
- Date: 2026-07-18
- Responsibility correction: 2026-07-18
- Decision owner: MFM platform owner
- Evidence: [slow-build baseline](../slow-build-baseline.md), R2-01 through R2-09
- Experimental history: [RFC_SLOW_BUILDS.md](../../RFC_SLOW_BUILDS.md)

## Context

MFM needs fast focused development, complete local verification, reproducible
packaging, and understandable artifact ownership. These workloads do not have
the same artifact semantics:

- direct development benefits from incremental, worktree-local Cargo state;
- broad verification needs the live workspace, all 974 Nextest tests,
  doctests, Trybuild, SQLx, services, and parity evidence; and
- packaging needs an immutable release output, not reusable test state.

R2-09 found that retaining Cargo's verification target was the only
performance-qualified candidate. Its clean median was 359.204s and its warm
median was 197.644s, a 161.560s/45.0% reduction, while preserving the complete
verification surface. The stable target occupied approximately 10.49 GB and
remained sensitive to worktree paths.

The original decision treated those operational properties as a request for a
Nixfied cache subsystem. The accepted upstream cache/port RFC corrected that
responsibility: compiler artifacts are an invocation-specific concern owned by
Cargo and MFM, while Nixfied owns execution, services, registry state, cleanup
of its own state, and execution evidence.

## Decision

MFM keeps Cargo as the only Rust unit-graph and compiler-artifact authority and
adds no compiler cache.

The platform has three explicit lanes:

1. Development uses the Nix-pinned shell and direct Cargo with incremental
   compilation in the worktree's normal `target` directory. `nix develop` and
   `.#quick` explicitly unset `CARGO_TARGET_DIR`.
2. Broad local verification uses the compact nonincremental profile in the
   project-owned `target/verification` directory. Nixfied tasks set this as an
   ordinary child environment variable; Nixfied does not interpret or manage
   it.
3. Release packaging remains the Nix `buildRustPackage` derivation. Its output
   is immutable and is neither a verification artifact nor a test cache.

One worktree has one verification target shared by all Nixfied slots. Separate
worktrees isolate naturally by filesystem path. Cargo owns fingerprints,
writer locking, corruption recovery, and rebuild decisions inside the target.
MFM owns its placement, inspection, retention, and deletion policy.

## Artifact authority

| Surface | Authority | Contract |
| --- | --- | --- |
| Rust and native tools | Nix | Pinned per supported platform; immutable store closures |
| Developer artifacts | Cargo and the developer | Mutable, incremental, normal worktree `target` |
| Verification artifacts | Cargo and MFM | Mutable, compact, worktree-owned `target/verification` |
| Verification results | Nixfied run evidence | Every selected task executes; never inferred from artifact presence |
| Services and runtime state | Nixfied | Registry, lifecycle, logs, summaries, and marker-gated cleanup |
| Release package | Nix | Immutable `buildRustPackage` output from locked source inputs |
| Hosted job execution | CI provider | Runs repository gates; no persistent compiler cache is selected here |

Cargo targets are accelerators, not trusted deliverables. Artifact presence
never substitutes for task execution or evidence.

## Placement and lifecycle

`CARGO_TARGET_DIR=target/verification` remains the ordinary authored value.
Each Cargo leaf makes it absolute from the invocation root before executing
Cargo, so nested tools such as Trybuild and the crate-local SQLx script inherit
one stable path before changing directories. Nixfied does not interpret this
path rule.

`NIXFIED_STATE_DIR` selects Nixfied runtime state and evidence only. It neither
selects nor cleans Cargo artifacts. Likewise, `nixfied clean` does not touch
`target/verification`.

Normal artifact cleanup is project-owned:

```sh
cargo clean --target-dir target/verification
```

Deleting a worktree also deletes its verification target. MFM may add an
independent retention policy later if new capacity evidence justifies one, but
that policy will not become a Nixfied model or runtime contract.

## Concurrency and trust

Broad verification leaves may share one worktree target. Cargo's own locking
serializes conflicting builds; `.config/nextest.toml` also keeps nested
Trybuild Cargo suites in one test group to avoid wasting runner time on the
same lock. Nixfied does not add a cache lease or synthesize reuse evidence.

The target is local, same-user mutable state. Cross-user sharing, remote
upload/download, signing, credentials, and binary-cache trust are outside this
decision. Hosted CI remains cold and ephemeral unless a separate CI-provider
cache decision establishes its trust and failure contract.

## Endpoint responsibility

The measured port-collision defect was a Nixfied runtime concern because
deterministic service endpoints are runtime-owned OS resources. The upstream
endpoint acquisition contract now coordinates same-user starts across roots,
verifies exact listener ownership, and reports typed conflict evidence before
service-specific mutation. This fix is independent of Cargo target placement:
changing `NIXFIED_STATE_DIR` does not make a colliding host port safe.

## Gate and evidence governance

The public gates remain `.#check`, `.#test`, `.#test-db`, and `.#ci`. Broad
gates use `target/verification`; direct development and `.#quick` use the
ordinary target. Runtime output reports execution progress, results, logs, and
artifact pointers. It carries no compiler-cache identity or hit/miss evidence.

The existing repository policy still requires the three component gates
before a commit and `.#ci` for final merge readiness. Focused Cargo commands
remain the normal development feedback loop.

## Rejected alternatives

- `sccache`: rejected because clean runs regressed 17.4% and second-worktree
  reuse missed materiality despite correct tested coverage.
- Crane verification artifact: rejected because its warm full-gate median
  improved only 4.4%/9.05s and retained substantial mutable Trybuild state.
- crate2nix and cargo2nix: rejected because neither preserved Cargo's complete
  dependency, feature, Trybuild, doctest, and parity semantics.
- execution-topology changes: rejected because no measured end-to-end case met
  both the 15% and 30-second materiality thresholds.
- a Nixfied compiler-cache contract: rejected because placement, writer
  locking, retention, inspection, bypass, and recovery belong to Cargo/MFM,
  not the generic execution runtime.

## Upgrade and rollback

The Nixfied pin and authored model are upgraded together; mixed ABIs are not
supported. Optional Nixfied-state cleanup must be performed with the old exact
runtime before repinning. The new runtime does not migrate old registry or
state layouts.

The verification target needs no framework migration. If it is suspected or a
cold comparison is required, MFM deletes exactly `target/verification` with
Cargo's cleanup command and reruns the same gates. Developer Cargo state and
Nix release packaging are unaffected.

## Consequences

The decision preserves one Rust dependency graph, exact existing coverage,
simple developer behavior, and ordinary Nix packaging without growing a cache
protocol in the model seam. Cold verification still takes several minutes,
each active worktree may retain roughly 10.5 GB, and execution/services
dominate warm gates. Those are explicit project-owned operating costs.
