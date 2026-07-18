# Nixfied capability handoff for MFM verification caches

Status: MFM draft ready for owner review; approval required before transmission

Date: 2026-07-18

Audience: Nixfied architects and MFM platform owners

## Purpose

MFM selected its long-term Rust build boundary in
[ADR 0001](adr/0001-mfm-rust-build-architecture.md). Cargo remains the only
Rust unit-graph authority. Nix pins tools and builds release packages. Nixfied
executes broad verification, manages services and evidence, and owns the
lifecycle of its mutable verification target.

This document requests only framework capabilities consumed by that selected
architecture. It does not ask Nixfied to compile Rust, reproduce Cargo's unit
graph, cache successful tests, transport remote artifacts, or support the
rejected sccache, Crane, crate2nix, or cargo2nix pilots.

## Exact downstream identity

The handoff is based on this committed MFM configuration:

| Input | Identity |
| --- | --- |
| Nixfied revision | `b0681e45ab76d5023d9c5e033087d34adf98e90b` |
| Nixfied NAR hash | `sha256-E2usrYJbA88cz1HgRo9TBvo5p2S6Mi6ke14g2/crk2M=` |
| Runtime ABI | `nixfied-runtime-abi:1-5ff3aa14f2bf` |
| Model hash | `d25c647af500ec8c060e173f05e199564cf3d3921663c82f04c3477a4c49ce84` |
| Toolchain ID | `nixfied-toolchain:1` |
| Reference target | `aarch64-linux`; Rust/Cargo 1.96.0 |
| Cache family/mode/scope | `cargo-target` / `fast-dev` / `slot` |
| Cache digest | `dd27515db6a0fc48aabe31c5e300af7325e3af24a98f0560364fa9c516971e03` |
| MFM policy key | `cargo-target-v2`, platform, Rust 1.96.0, `verification-v2` |

The current `invocation.cacheEnv.CARGO_TARGET_DIR` support already validates
path-safe family and key components, includes target/toolchain/runtime identity
in the digest, materializes below the runtime-owned cache root, and reports the
resolved cache environment in task summaries. The requested work extends that
foundation; MFM does not need a replacement cache declaration model.

## Selected consumer and policy

The only consumer is MFM's broad local verification lane: `.#check`, `.#test`,
`.#test-db`, and `.#ci`. Its target is mutable Cargo state, not an authoritative
artifact. Every selected test and check still executes.

The accepted MFM policy is:

- one namespace per project, codebase, worktree, slot, platform/target,
  toolchain, runtime/cache schema, and verification policy;
- exclusive writer ownership within a namespace;
- 20 GiB maximum per Cargo-target identity;
- 32 GiB maximum for MFM's Cargo-target family;
- 14 days maximum idle age;
- inactive least-recently-used eviction;
- an explicit cacheless/bypass path; and
- cache-only cleanup that preserves services and evidence.

MFM expects these values to be declarative downstream policy, not Nixfied
global defaults.

## Required for rollout (P0)

Every P0 below has a current failing MFM scenario, a selected consumer, and a
framework-level acceptance test. Durable rollout is blocked until all five
contracts exist in one accepted Nixfied release.

### P0-1: worktree namespace and exclusive writer ownership

Consumer: every Cargo leaf in the broad verification lane.

Current failing scenarios:

- The cache is scoped by state root and slot, not enforced worktree identity.
  An exact second worktree reused the same target, increased full CI by 56.516s
  against the R2-09 warm median, and added approximately 404 MB of path-specific
  artifacts. R2-04 independently observed approximately 475 MB of added
  Trybuild state.
- Two same-state, same-slot `.#test` invocations can enter the same target.
  Cargo may lock compilation internally, but Nixfied does not own or expose an
  exclusive writer lease. R2-09 gate retries observed SIGTERM failures while
  unrelated agents overlapped the namespace.

Required contract:

- Derive or require a stable worktree namespace in addition to project,
  codebase, environment, slot, and cache identity.
- Refuse accidental omission or collision; do not silently share a target
  between worktrees or fall back to an unowned path.
- Acquire an exclusive writer lease before launching a cache-consuming leaf.
  A second writer waits with bounded diagnostics or fails with a typed conflict.
- Bind leases to registered processes and reconcile them after success,
  failure, cancellation, or runtime restart.
- Report worktree namespace, lease owner, wait/refusal outcome, and cache
  identity without exposing sensitive filesystem contents.

Framework acceptance test:

1. Create two checkouts of one fixture at the same revision.
2. Run equivalent cached leaves in the same slot and prove distinct cache
   paths and ownership records.
3. Start two writers in one worktree/slot and prove only one child executes at
   a time, or the second fails with a stable conflict code naming the owner.
4. Cancel the first writer, reconcile the registry, and prove a later writer
   can acquire the lease and complete using the recovered identity.
5. Prove two different slots remain isolated independently of worktree names.

Evidence: R2-04 worktree/concurrency/cancellation matrix, R2-09 exact-second-
worktree run, and ADR 0001 ownership contract.

### P0-2: inspectable identity, lifecycle evidence, and explicit bypass

Consumer: MFM verification operators, rollback procedures, and gate evidence.

Current failing scenarios:

- Task summaries report family, digest, mode, scope, and path, but not size,
  creation/last-use time, worktree owner, policy version, active lease, retained
  identities, or an eviction decision. R2 measurements required direct
  filesystem traversal.
- The pinned runtime has no explicit bypass for the same task and namespace.
  A fresh state root is only a cacheless proxy and changes ownership/placement.

Required contract:

- Provide a read-only inspection surface for project/cache family/identity
  showing identity inputs, resolved path, owner namespace, size, creation and
  last-use times, policy version, state, and active leases.
- Record creation, reuse, bypass, over-budget marking, eviction, cleanup,
  refusal, cancellation recovery, and corruption recovery in structured run or
  lifecycle evidence.
- Provide an explicit task/run bypass that substitutes a fresh ephemeral cache
  while preserving source, toolchain, profile, features, services, and task
  graph. The bypass must not mutate or borrow the durable identity.
- Keep public diagnostics free of environment values, credentials, and child
  file names.

Framework acceptance test:

1. Run one cached leaf twice and prove inspection changes from created to
   reused with stable identity and last-use evidence.
2. Run the same leaf through bypass and prove a distinct ephemeral path,
   identical command/task inputs, no durable-cache mutation, and explicit
   bypass evidence.
3. Hold a lease and prove inspection identifies active ownership.
4. Cancel the task and prove reconciled lifecycle evidence and a reusable or
   explicitly evictable identity.
5. Verify JSON diagnostics contain no fixture secret or injected environment
   value.

Evidence: R2-04 inspection and bypass gaps, R2-09 cacheless matrix, and ADR
0001 evidence/rollback contract.

### P0-3: bounded retention and deterministic admission

Consumer: the durable Cargo target selected for broad local verification.

Current failing scenario:

The target was approximately 10.49 GB when stable. Cross-worktree and changed-
policy variants accumulated without automatic pruning; R2-04 reached
16,924,336,128 bytes under one identity, and repeated isolated worktrees
multiplied the full target. The runtime exposes no age/size budget or inactive
identity selection.

Required contract:

- Admit declarative per-identity byte, per-family byte, and idle-age limits.
- Calculate policy over runtime-owned identities without downstream directory
  traversal.
- Before admitting new growth, select inactive identities deterministically by
  least recent use and expose the proposed plan through dry-run inspection.
- Never prune an active lease. Mark an identity that crosses its limit while
  active as over budget; require safe eviction before its next lease.
- Make interrupted retention recoverable and the final accounting auditable.

Framework acceptance test:

1. Configure small fixture limits and create three identities with controlled
   sizes and last-use times.
2. Dry-run and prove the exact inactive LRU plan, reclaimed-byte estimate, and
   reason codes.
3. Hold a lease on the oldest identity and prove it is skipped or admission
   fails closed rather than deleting it.
4. Execute retention twice and prove deterministic, idempotent accounting.
5. Cross an identity limit during an active write, prove over-budget evidence,
   then prove the next lease evicts/recreates it cold.
6. Change only age/size policy and prove cache contents are not silently
   reinterpreted as another identity.

Evidence: R2-04 growth/retention matrix, R2-09 persistent-byte comparison, and
ADR 0001's 20/32 GiB and 14-day policy.

### P0-4: cache-only cleanup with path and lease safety

Consumer: operator cleanup, retention execution, corruption recovery, and
architecture rollback.

Current failing scenario:

`nix run .#clean -- --slot 0` deletes the entire slot state root. R2-04 and
R2-09 used this supported operation only after evidence extraction; it removed
Cargo targets together with run summaries, task logs, parity artifacts, and
service state. Active-process refusal is sound, but the cleanup scope is too
broad for routine cache lifecycle.

Required contract:

- Add a first-class cache-family/exact-identity cleanup or pruning operation;
  do not model it as a project shell task.
- Confine deletion below a marker-owned runtime cache root. Refuse path
  escapes, symlink components, marker mismatch, malformed identities, and
  active or unreconciled leases/processes.
- Preserve registry records, run summaries/logs, parity artifacts, Postgres
  data, service state, and unrelated cache families.
- Support dry-run, idempotent repeated execution, exact deleted paths/bytes,
  stable refusal codes, and interrupted-cleanup recovery evidence.

Framework acceptance test:

1. Create one slot containing two cache families, Postgres data, a service
   record, run logs/summaries, parity artifacts, and registry state.
2. Clean one exact cache identity and prove every non-target surface and the
   other cache remain byte-identical.
3. Repeat cleanup and prove success with zero additional deletion.
4. Refuse cleanup for an active lease, symlink, escaped path, bad marker,
   malformed digest, and unreconciled live process using typed diagnostics.
5. Interrupt cleanup between plan and commit, reconcile, and prove the result
   is either safely completed or safely retryable without broad deletion.

Evidence: R2-04 cleanup/refusal experiment, R2-09 cleanup IDs and deleted run
evidence, and ADR 0001 cleanup/rollback contract.

### P0-5: globally actionable slot and port collision diagnostics

Consumer: `.#test-db` and `.#ci`, whose selected verification graph starts
managed Postgres.

Current failing scenario:

Independent state roots using slot 0 both select `127.0.0.1:28080`. R2-04's
second process failed with `PROC_ESCAPE` and PostgreSQL's address-in-use text.
State-root isolation therefore does not provide host-global service placement
ownership.

Required contract:

- Detect or reserve the derived host/port placement across runtime state roots
  before launching the service child.
- On conflict, fail with a stable placement code that reports project,
  environment, slot, service, endpoint, and live owning process/run when known.
- Do not expose service credentials or require scanning arbitrary process
  command lines.
- Reconcile stale reservations after confirmed process death.

Framework acceptance test:

1. Start the same managed service in two state roots with the same slot.
2. Prove the second fails before child escape with a typed collision naming the
   first owner and endpoint.
3. Stop the owner and prove the second root can acquire placement.
4. Simulate a stale registry owner and prove reconciliation distinguishes it
   from a live collision.
5. Start different slots and prove their endpoints remain distinct.

Evidence: R2-04 port isolation failure and ADR 0001's globally actionable
collision requirement.

## Security and lifecycle invariants

The five P0 contracts share these non-negotiable invariants:

- Runtime-owned markers and canonicalized confinement are checked before any
  mutation; symlinks are never followed for inspection or deletion.
- No cleanup, retention, bypass, or recovery action mutates an active cache
  lease or another worktree/slot namespace.
- Registry/process reconciliation precedes destructive action and fails closed
  when liveness is ambiguous.
- Lifecycle records are append-only/auditable enough to explain identity,
  owner, policy, action, refusal, and reclaimed bytes.
- Cache operations never delete or rewrite service state, run evidence, or
  unrelated cache families.
- Public output never includes credentials, environment values, source file
  contents, child cache file names, or other secret-bearing diagnostics.
- A cache hit never substitutes a successful task outcome; children execute on
  every authoritative gate.
- Failure leaves either the prior identity usable, the identity explicitly
  marked for eviction, or an idempotent operation that can be reconciled.

## Optional future improvements (P1)

These are useful to the selected operator workflow but do not block rollout:

| Capability | Selected consumer | Boundary |
| --- | --- | --- |
| Cross-slot retention planner | MFM operators managing the 32 GiB family budget | Aggregate existing P0 inspection into one dry-run plan; no new deletion semantics |
| Historical size/eviction trends | Capacity planning for local verification | Retain bounded aggregate lifecycle metrics without enumerating child files |
| Model-policy diagnostics | MFM policy review | Explain effective worktree/slot/age/size policy and why an identity differs before execution |

P1 work must reuse P0 identity, safety, and evidence contracts. It must not
delay the minimal rollout implementation.

## Explicit non-requests

The selected architecture has no consumer for these capabilities, so they are
not part of this handoff:

- task-specific immutable verification closures or eager/lazy realization
  controls—the Crane consumer was rejected, and normal Nix package laziness is
  sufficient for selected packaging;
- sccache daemon ownership, compiler-object hit reporting, or remote backends;
- crate2nix/cargo2nix graph generation, drift checks, overrides, or test
  runners;
- Cargo unit-graph, fingerprint, feature, build-script, proc-macro, rustdoc,
  linking, or source-input semantics;
- test-result caching or suppression of Nextest, doctest, Trybuild, SQLx,
  keystore, CLI, REST, metadata, or Postgres parity execution;
- cross-user trust, artifact signing, remote transport, credentials, or hosted
  cache persistence; and
- internal pruning of Cargo units inside one target. Nixfied owns identity-
  level lifecycle; Cargo owns target contents.

## MFM-owned responsibilities

MFM will own and test:

- the pinned Nixfied, Nixpkgs, Rust/Cargo, Nextest, and SQLx identities;
- `nixfied.nix` task graph, public verbs, services, cache declaration, profile,
  policy version, platform key, and 20/32 GiB plus 14-day values;
- the direct incremental developer target and its worktree-local convention;
- Cargo manifests, lockfile, features, fingerprints, source inputs, and
  compilation behavior;
- the exact 99-binary/974-test coverage inventory and all non-Nextest leaves;
- release packaging through `buildRustPackage`;
- local same-user trust policy and the decision not to import mutable targets;
- repository gate governance and any later exact `.#ci` simplification; and
- platform correctness validation and documented rollback during rollout.

MFM will not add a shell cache cleaner, manually traverse runtime-owned cache
contents in ordinary operation, invent a lease registry, or implement a
project-local workaround for a missing P0.

## Minimal downstream reproductions

The framework team can reproduce every gap with a small fixture; the MFM
workspace is not required for acceptance tests.

| Gap | Minimal fixture |
| --- | --- |
| Worktree ownership | Two checkouts, one cached leaf, same project/environment/slot, concurrent and sequential runs |
| Inspection/bypass | One leaf that writes a visible artifact and records an injected secret sentinel that must remain redacted |
| Retention | Three cache identities with deterministic sizes/timestamps and one held lease |
| Cache-only cleanup | One slot with two caches, service data, registry, run logs, and a symlink/path-escape adversary |
| Port collision | One TCP service, two independent state roots, same slot/derived endpoint |

For downstream parity, MFM will additionally rerun `.#check`, `.#test`,
`.#test-db`, and `.#ci`; verify the frozen Nextest identifier digest
`e75d6ea039b5507c6f9b89bef89656e31073c02f8f17f74f680fc2bbf0d67f08`;
exercise exact, changed-source, second-worktree, bypass, cancellation, cleanup,
and over-budget cases; and confirm service/run evidence survives cache cleanup.

## Delivery and decision boundary

MFM owner approval is required before this document is sent to Nixfied's
architects. After their response, MFM will create a separate implementation
plan that pins the accepted framework revision, declares the selected policy,
tests Linux and macOS correctness, proves rollback, and updates gate governance
only if exact task/evidence equivalence remains true.

This handoff is complete when Nixfied accepts, revises, or explicitly declines
each P0 contract. It does not authorize MFM rollout or a local workaround.
