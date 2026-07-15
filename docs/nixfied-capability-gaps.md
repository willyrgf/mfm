# Nixfied capability gaps for MFM

This document tracks Nixfied capabilities MFM needs before it can safely
replace the current raw Cargo verification target with a first-class scoped
cache. It is written for discussion with the Nixfied team and is intentionally
separate from the build-baseline report.

## Context

MFM is pinned to Nixfied revision `b0681e45ab76d5023d9c5e033087d34adf98e90b`
with runtime ABI `nixfied-runtime-abi:1-5ff3aa14f2bf`.

The current MFM verification lane uses the Phase 05 artifact policy and a
Nixfied `invocation.cacheEnv` identity for the `cargo-target` family. The
runtime can materialize that identity, but it cannot clean one cache family
without deleting the whole slot state root. A broad slot cleanup would also
remove Postgres data, run records, logs, and parity diagnostics.

## Requested capabilities

| Priority | Capability | Required behavior | Acceptance evidence |
| --- | --- | --- | --- |
| P0 | Cache-family garbage collection | Delete only a declared cache family and selected cache identities under the runtime-owned state root. Enforce marker ownership, path confinement, active-lease/process checks, symlink refusal, idempotence, and an auditable cleanup result. | A cache-only cleanup removes Cargo artifacts while preserving `pgdata`, run records, logs, parity artifacts, registry state, and unrelated cache families. Repeated cleanup is safe. |
| P0 | Cache cleanup lifecycle surface | Expose a first-class runtime operation or control verb for cache-family cleanup/pruning. It must not be implemented by a project shell task, `find -delete`, or broad slot deletion. | Model admission, runtime execution, refusal, and cleanup evidence are covered by framework tests and usable by downstream projects. |
| P0 | Explicit cache identity contract | Keep `cacheEnv` identity stable and documented: family, mode, scope, logical key parts, target identity, runtime ABI, and toolchain identity. Support an explicit policy-version key. | Changing platform/target, compiler/toolchain, or policy version produces a different cache identity; unchanged sequential runs reuse the same identity. |
| P0 | Worktree/concurrency boundary | Make the cache namespace unambiguously isolated by slot and concurrent worktree, either through a declared worktree identity or a runtime-enforced per-worktree state root. | Two worktrees and two slots can run concurrently without sharing Cargo target files or registry ownership. The evidence exposes the selected namespace. |
| P1 | Cache leases or locking | Provide an optional lease/lock for shared slot caches, or document and enforce refusal semantics for concurrent writers. | Concurrent same-slot tasks cannot corrupt a Cargo target; one task waits or fails with a typed, actionable diagnostic. |
| P1 | Cache inspection and accounting | Expose cache family, identity, path, age, size, owner, and active-use information without reading child contents. | A report can measure cache growth and select stale identities without traversing service or diagnostic state. |
| P1 | Bounded retention policy | Support safe age/size limits for a cache family, with dry-run and explicit execution modes. | Pruning is deterministic, bounded, marker-gated, auditable, and never follows symlinks or escapes the owned cache root. |
| P2 | Lifecycle compatibility evidence | Include cache creation, reuse, invalidation, cleanup, refusal, and recovery evidence in the normal runtime summaries and registry records. | Downstream projects can prove which cache identity a task used and why cleanup was accepted or refused. |

## Existing support that MFM can use

The current `cacheEnv` mechanism is useful and should remain the foundation:

- cache declarations are restricted to leaf task invocations;
- cache families and key parts are validated as path-safe components;
- cache paths are materialized below a runtime-owned `caches/<family>` root;
- the digest includes the cache schema, family, mode, scope, key parts, target
  identity, runtime ABI, and toolchain identity; and
- cache paths are injected into selected child environment variables.

The missing piece is lifecycle ownership: the runtime has no cache-family
cleanup operation, while the existing slot `clean` operation owns the entire
slot state root.

## MFM's safe interim behavior

Until the requested support exists:

- use `.#quick` and the repository-pinned developer shell for fast local
  iteration;
- keep the Phase 05 Nixfied verification lane as the authoritative gate lane;
- use the existing slot lifecycle only when broad slot cleanup is intentional;
- measure experimental cache identities in disposable, isolated targets; and
- for the explicitly approved Phase 06 pilot, manually remove only the exact
  reported Cargo cache digest after recording its path and preserving the
  surrounding state; and
- keep this manual operation as a measurement-only exception, not as a
  reusable project cleaner or a substitute for framework lifecycle semantics.

MFM must not add a shell-level Cargo cache cleaner, broad recursive deletion,
or a second ad hoc target path to simulate the missing framework capability.
Phase 06 is explicitly re-scoped to permit the exact-path measurement exception
above; the upstream cache lifecycle request remains open. Phase 07 may proceed
under that owner-approved exception, with the limitation recorded in its report.

## Suggested framework test matrix

1. Run one cached leaf twice in one slot and prove the same cache identity is
   reused.
2. Run equivalent leaves in two slots and two worktree namespaces and prove
   the paths and ownership records are distinct.
3. Change the target platform, runtime/toolchain identity, and policy-version
   key independently and prove each change invalidates the identity.
4. Create a cache plus Postgres state, run records, logs, and unrelated cache
   families, then perform cache-only cleanup and prove only the selected cache
   family was removed.
5. Refuse cleanup with active leases/processes, mismatched markers, symlinked
   roots, path escapes, and malformed identities.
6. Exercise repeated cleanup, interrupted cleanup recovery, dry-run pruning,
   size/age bounds, and registry/audit evidence.

## Decision status

Phase 06 of [RFC_SLOW_BUILDS.md](../RFC_SLOW_BUILDS.md) is re-scoped under an
owner-approved exact-path measurement exception. MFM will keep this list
current as the Nixfied team responds; the P0 lifecycle capabilities remain
needed before manual cleanup can become an ordinary project operation.
