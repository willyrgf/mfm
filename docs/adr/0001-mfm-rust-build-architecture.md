# ADR 0001: MFM Rust build architecture

- Status: accepted
- Date: 2026-07-18
- Accepted: 2026-07-18
- Decision owner: MFM platform owner
- Evidence: [slow-build baseline](../slow-build-baseline.md), R2-01 through R2-09
- Governing plan: [RFC_SLOW_BUILDS.md](../../RFC_SLOW_BUILDS.md)

## Context

MFM needs fast focused development, complete local verification, reproducible
packaging, and bounded artifact ownership. These workloads do not have the same
artifact semantics:

- direct development benefits from incremental, worktree-local Cargo state;
- broad verification needs the exact live workspace, all 974 Nextest tests,
  doctests, Trybuild, SQLx, services, and parity evidence; and
- packaging needs an immutable release output, not reusable test state.

R2-09 found only the existing scoped Cargo verification target
performance-qualified. Its clean median was 359.204s and its warm median was
197.644s, a 161.560s/45.0% reduction. It preserved the complete verification
surface. The target is approximately 10.49 GB when stable, is sensitive to
worktree paths, and can grow without a first-class retention boundary.

`sccache`, Crane, crate2nix, cargo2nix, and execution-topology changes each
failed an independent performance or correctness screen. Combining rejected
components has no approved additive hypothesis.

## Decision

MFM keeps Cargo as the only Rust unit-graph authority and adds no new compiler
cache.

The platform has three explicit lanes:

1. Development uses the Nix-pinned shell and direct Cargo with incremental
   compilation in the current worktree's normal target.
2. Broad local verification uses the current compact, nonincremental Cargo
   policy in a durable Nixfied-owned target. Durable rollout is conditional on
   the lifecycle prerequisites below.
3. Release packaging remains the Nix `buildRustPackage` derivation. Its output
   is an immutable package and is not a verification artifact or test cache.

Nix pins tools and builds immutable packages. Cargo resolves, fingerprints,
compiles, links, and runs the live Rust graph. Nixfied declares and executes
verification tasks, owns mutable verification placement and lifecycle, manages
services, and records evidence. The CI provider invokes those public gates on
supported platforms.

Until the lifecycle prerequisites are implemented and reviewed, the existing
verification target remains provisional. Broad slot cleanup is allowed only
when broad slot deletion is intended; it is not the selected cache-retention
mechanism.

## Artifact authority

| Surface | Authority | Contract |
| --- | --- | --- |
| Rust and native tools | Nix | Pinned per supported platform; immutable store closures |
| Developer artifacts | Cargo and the developer | Mutable, incremental, worktree-local, non-authoritative |
| Verification artifacts | Cargo for contents; Nixfied for placement/lifecycle | Mutable, compact, isolated by project/worktree/slot/policy |
| Verification results | Nixfied run evidence | Never inferred from artifact presence; every selected test and check executes |
| Services and parity state | Nixfied | Separate from compiler cache lifecycle |
| Release package | Nix | Immutable `buildRustPackage` output from locked source inputs |
| Hosted job execution | CI provider | Runs repository gates; does not become an artifact authority |

Cargo targets are accelerators, not trusted deliverables. A cache hit cannot
replace test execution, and mutable target contents are never imported from an
untrusted user, repository, archive, or remote backend.

## Identity and input contract

The verification namespace must identify at least:

- project and codebase;
- enforced worktree identity;
- slot;
- host platform and Rust target;
- Rust/Cargo toolchain identity;
- verification profile and policy version;
- Nixfied cache schema and runtime ABI; and
- cache family and mode.

Cargo remains responsible inside that namespace for source, manifest,
lockfile, feature, build-script, proc-macro, environment, rustdoc, link, and
non-Rust input fingerprints. The namespace must not include the source digest:
safe changed-source reuse is part of Cargo's contract and was measured by the
RFC. A toolchain, target, cache schema, or verification-policy change must
select a new namespace.

The task summary must report the resolved identity, path, policy version,
worktree, slot, and whether the run reused, created, bypassed, or evicted it.

## Ownership, concurrency, and trust

- One verification namespace belongs to one local MFM worktree and slot.
- Two worktrees must not share target files, even at the same source revision.
- Verification leaves within one namespace may reuse the target sequentially.
- Nixfied must provide an exclusive writer lease or an equivalent enforced
  wait/refusal contract. Cargo's internal lock alone is not the platform
  ownership policy.
- A conflicting writer receives a typed diagnostic naming the namespace and
  owner; it never falls back to another mutable target silently.
- The selected cache is local and same-user. Cross-user sharing, remote
  upload/download, signing, credentials, and binary-cache trust are outside
  this decision.
- Slot-derived service ports need globally actionable collision diagnostics;
  a separate state root does not make a colliding port safe.

## Inspection, retention, and cleanup

Nixfied must expose cache-family inspection without requiring MFM to traverse
runtime-owned directories. Inspection reports identity, owner, path, size,
creation and last-use times, policy version, and active leases.

The initial MFM retention policy is:

- 20 GiB maximum per Cargo-target identity;
- 32 GiB maximum for the project's Cargo-target family;
- 14 days maximum idle age; and
- least-recently-used eviction of inactive identities before admitting new
  cache growth.

The 20 GiB identity budget covers the measured approximately 10.49 GB steady
target and the 16.9 GB worst experimental accumulation while bounding the
failure mode. The family budget supports normal worktree isolation without
allowing indefinite multiplication. These are declarative MFM defaults, not
hard-coded framework limits, and may change only with new capacity evidence.

An identity that crosses its budget during an active lease is marked over
budget and reported. It is not mutated underneath the writer; the next lease
must evict it safely and rebuild cold. Cleanup and pruning must:

- support dry-run and explicit execution;
- operate on cache families or exact identities only;
- refuse active leases, symlinks, marker mismatches, and path escapes;
- be idempotent and auditable; and
- preserve Postgres data, service state, run records, logs, registry records,
  parity artifacts, and unrelated cache families.

MFM will not implement this contract with a shell cleaner or recursive
filesystem deletion.

## Cacheless, failure, and recovery behavior

An explicit Nixfied bypass must run the same task with a fresh ephemeral Cargo
target while preserving toolchain, profile, source, features, services, and
coverage. Bypass is diagnostic and rollback behavior, not a second normal
cache lane.

Cache absence and eviction are normal misses followed by a cold build. Identity
or ownership ambiguity fails closed. A canceled or failed Cargo process
releases its lease; a later run may reuse the target only after process
reconciliation. Suspected corruption is handled by exact-identity eviction or
bypass and a cold rebuild, with the action recorded. No successful test result
is cached.

There is no selected remote backend, so backend-unavailable behavior is not
applicable. Adding one requires a separate trust, transport, credential,
failure, and performance decision.

## Evidence and gate governance

The public gates remain `.#check`, `.#test`, `.#test-db`, and `.#ci`. The `ci`
composite directly contains the other public task graphs with run-once
semantics and adds keystore parity. Therefore one successful `.#ci` run is
evidence-equivalent to separately rerunning the three component gates when all
of the following are identical:

- exact candidate source tree and lockfile digest;
- compiled Nixfied model and task graph;
- toolchain, target, profile, features, and cache policy; and
- required service and coverage selection.

Gate-policy simplification is selected but not activated by this ADR. A later
rollout must update repository policy and prove the exact task/evidence mapping
in the same change. Until then, the current requirement to run `.#check`,
`.#test`, `.#test-db`, and final `.#ci` remains authoritative. Focused Cargo
checks remain the normal development feedback loop.

## Portability and hosted adoption

The authority and lifecycle contract is portable across Linux and macOS, but
artifacts and identities are platform-specific. R2 performance qualification
applies only to the reference aarch64-linux host.

The existing hosted Linux and macOS lanes remain correctness guards. A later
rollout may enable the same local target lifecycle on either platform only
after its public gates pass and any platform-specific implementation difference
is documented. Ephemeral hosted runners may continue cold until persistence is
separately justified. Linux timings do not predict hosted or macOS timings,
and Rust artifacts are never shared across incompatible targets.

## Execution topology

No execution-topology implementation proceeds from this RFC. The measured
worker, doctest-overlap, and parity-overlap cases all missed the simultaneous
15% and 30-second end-to-end threshold. A separate plan requires new residual
evidence and must preserve target ownership, service/schema isolation, and
deterministic diagnostics.

## Rejected alternatives

- `sccache`: rejected because clean runs regressed 17.4% and second-worktree
  reuse missed materiality despite correct tested coverage.
- Crane verification artifact: rejected because its warm full-gate median
  improved only 4.4%/9.05s and retained substantial mutable Trybuild state.
- crate2nix: rejected because it could not represent required workspace inputs
  or expose a supported authoritative verification artifact.
- cargo2nix: rejected because its dependency/feature graph diverged from Cargo,
  its locality was repository-wide, and complete Trybuild coverage failed.
- execution-topology changes: rejected because no measured end-to-end case met
  both materiality thresholds.
- combined candidates and remote caches: not selected because no standalone
  components supplied a qualifying additive hypothesis and no remote trust or
  transport evidence exists.

## Rollback

The architecture has a low-cost rollback because Cargo remains authoritative.
If durable lifecycle support is faulty, MFM disables persistence through the
explicit bypass, evicts only the affected identity after leases close, and
runs the unchanged public gates cold. Developer Cargo and Nix release packaging
are unaffected. No verification schema, output, or test contract changes.

## Rollout prerequisites

R2-11 must turn this decision into a minimal Nixfied architect handoff. Durable
rollout requires framework-level acceptance tests for:

1. worktree/slot identity and exclusive writer ownership;
2. inspection and accounting;
3. configurable age/size retention and deterministic inactive eviction;
4. cache-only cleanup with active-lease and path-safety refusal;
5. explicit bypass, cancellation recovery, and lifecycle evidence; and
6. actionable slot/port collision diagnostics.

MFM-owned rollout then needs a separate, small plan to pin the accepted
Nixfied revision, declare the policy, update gate governance if approved, test
Linux and macOS correctness, and document rollback. This ADR does not enable
those changes.

## Consequences

The decision preserves one Rust dependency graph, exact existing coverage,
simple developer behavior, and ordinary Nix packaging. It avoids a new daemon,
generated graph, verification archive, and remote trust boundary.

The cost is deliberate: cold verification remains several minutes, each active
worktree may own roughly 10.5 GB, execution and services dominate warm gates,
and durable verification remains blocked on Nixfied lifecycle support. Those
costs are measured and bounded by the selected contract rather than hidden by
an unqualified cache.
