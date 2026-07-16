# RFC: evidence-led Rust build architecture for MFM

Status: proposed revision; completed remediation retained, architecture experiments pending

Date: 2026-07-16

This revision supersedes the previous forward plan in this file. The results of
the original Phases 00–07 remain valid evidence. The attempted Phase 08 was
reverted, and the old Phases 09–17 are withdrawn.

## Executive decision

MFM will not decide its long-term Rust cache architecture before testing the
remaining credible alternatives against the same workloads.

Until that decision, MFM uses one pinned build environment with two artifact
lanes:

1. developers enter the Nix-provided environment and let Cargo own fast,
   incremental, worktree-local compilation; and
2. comprehensive verification runs through Nixfied, which lets Cargo compile
   the live workspace under a compact, isolated verification policy and owns
   services, task ordering, state, and evidence.

This is not a Cargo environment competing with a Nix environment. Cargo runs
inside the Nix-pinned environment in both lanes. The split is between mutable
artifact policies and assurance levels, not between toolchains.

The long-term CI and verification strategy remains open. The candidates to
test are:

- the current scoped Cargo target on a persistent trusted runner;
- a bounded `sccache` object cache around the same Cargo build;
- a coarse Nix-native verification artifact built with Crane and consumed
  directly by Nixfied;
- granular per-crate Nix artifacts generated independently with `crate2nix`
  and `cargo2nix`; and
- the current target-only design if no added mechanism produces enough value.

Test execution and task scheduling are measured as an independent candidate
work stream because compilation is no longer the majority of every full gate.
MFM will test maintained per-crate Nix graph generators, but it will not adopt
one without artifact-consumption and full-surface parity evidence, write a
project-local Rust build framework, copy mutable targets out of `/nix/store`,
or cache a successful test result.

The experiments come first. A later decision phase will define the selected
architecture, ownership, trust, retention, rollback, and gate governance. Only
after those definitions are complete will the final phase update
[the Nixfied capability-gap handoff](docs/nixfied-capability-gaps.md) for
discussion with Nixfied's architects. The current handoff document remains a
provisional Phase 06 record and should not be sent as Revision 2's architecture
request before that final phase.

## Why the previous RFC stops here

The original RFC was valuable through Phase 07, but its later plan treated
Nix-native compilation as a sequence to implement rather than a hypothesis to
falsify. Phase 08 exposed that mistake: MFM generated stand-in dependency
sources and produced an output that no verification task consumed. The output
could exist and be cached without making any MFM gate faster or more correct.

The lesson is broader than that failed pilot:

- an artifact has no value until an authoritative or shadow workload consumes
  it;
- a successful derivation is not evidence of useful reuse;
- a dependency cache must be measured through the final artifact that consumes
  it;
- immutable compilation does not reduce test, service, or parity execution;
- per-crate derivations introduce a second representation of Cargo's build
  graph whose fidelity, reuse granularity, and maintenance cost must be
  measured; and
- choosing a cache before measuring real CI transfer and execution costs
  optimizes the mechanism rather than the platform.

The old Phase 08 implementation and its repository-owned stub generator were
removed. Nothing from that pilot is part of the current design. The old
Phases 09–17 have no approval or implementation status.

## Retained foundation

The replacement RFC does not discard successful work. It treats the following
results as the starting point documented in
[the controlled baseline](docs/slow-build-baseline.md).

| Original phase | Retained conclusion |
| --- | --- |
| 00–01: RFC and baseline | Keep. The work established a reproducible inventory, exposed an approximately 18 GiB verification target, quantified split-debug and incremental artifacts, and recorded a disk-full failure. |
| 02: duplicate app tests | Keep. One explicit workspace Nextest run preserves the same 974 identifiers while removing 70 duplicate executions and reducing the full graph from 14 to 13 tasks. |
| 03: SQLx preclean | Keep. The destructive package clean is gone, and the disposable schema mutation check proves online schema drift still fails closed. |
| 04: developer lane | Keep. The pinned development shell and `.#quick` convenience path provide one toolchain for direct Cargo development. `.#quick` is useful but is not the primary performance mechanism. |
| 05: compact artifacts | Keep. Verification storage fell from approximately 17–18 GiB to 9.8 GiB; incremental artifacts and `.dwo` files fell to zero while source-line diagnostics remained available. |
| 06: scoped target identity | Retain provisionally as the current baseline. It proved identity, isolation, and invalidation but produced no material speed or storage gain. Cache-family inspection, leases, retention, and cleanup remain incomplete in the pinned Nixfied runtime. |
| 07: profile experiment | Keep the negative result. The default profile beat both `codegen-units=256` and `opt-level=1`; neither setting belongs in the verification policy. |
| 08: dependency artifact | Reject and keep reverted. The custom generated stubs were not a faithful build boundary, and no verification artifact consumed the output. |

Completed remediation may be changed only with new evidence. It is not
reopened merely because this RFC resets the future phase plan.

## Current facts

### Build and assurance surface

The measured workspace has 53 packages and 100 Cargo targets. Its complete
assurance surface includes:

- 974 Nextest test identifiers across 99 binaries;
- 53 doctest binaries and 42 discovered doctests;
- nine trybuild harnesses, 80 Rust UI cases, and 71 checked stderr baselines;
- Cargo metadata and architecture contracts;
- offline and online SQLx checks, including deliberate schema mutation;
- six database/parity leaves under managed Postgres; and
- 13 tasks in the full Nixfied CI graph.

Counts are useful diagnostics, but exact identifiers and selected features are
the coverage contract. An experiment does not pass merely because its total
count is similar.

MFM also has compile inputs that are easy for a source filter or per-crate
builder to miss:

- build scripts observe migrations and `.sqlx` metadata;
- tests use `include_str!` across crate and binary boundaries;
- trybuild launches nested Cargo processes against UI sources and `.stderr`
  expectations; and
- SQLx online preparation observes a live schema that is not a normal compiler
  input.

These are correctness constraints for every cache experiment.

### What the timings say

The Phase 07 default-profile comparison recorded:

| State | Full CI | Nextest task | Nextest compile | Nextest execution | Parity duration sum |
| --- | ---: | ---: | ---: | ---: | ---: |
| Clean | 341.392s | 160.765s | 45.93s | 114.415s | 95.368s |
| Warm | 226.593s | 110.181s | 25.10s | 84.651s | 78.136s |

These are controlled reference-host observations, not promises for current
GitHub runners. Later committed-tree confirmations also showed substantial
host and service variance. Revision 2 therefore begins by refreshing the
baseline on the environments where a decision will apply.

The clean-to-warm difference above is 114.799 seconds, or approximately 33.6%
of the clean run. Even an impossible compilation cache with zero lookup,
transfer, linking, and realization cost could not remove the test and service
execution floor. Nextest execution alone exceeded its logged compile interval
in both runs. The parity durations add real cost as well, although concurrent
tasks mean their sum must not be added directly to wall time.

The correct optimization order is therefore:

1. preserve fast focused development;
2. remove governance or task-graph re-execution that provides no new evidence;
3. measure execution topology and compilation reuse separately;
4. choose the least complex qualifying mechanism; and
5. optimize another layer only when the residual profile still justifies it.

### What is and is not cached today

Nix currently provides the pinned Rust toolchain, Cargo Nextest, SQLx CLI,
native tools, Nixfied runtime, model, launchers, and packaged CLI closure.
Nixfied's `cacheEnv` gives verification Cargo a stable runtime-owned target
path. It is a placement and identity mechanism, not result memoization; each
leaf still executes.

The public verification launchers are Nix store outputs, but their Cargo
artifacts are not. At runtime Cargo compiles the live checkout into mutable
Nixfied state. Conversely, the packaged `mfm` CLI is a real Nix derivation, but
it builds a release binary from the whole repository and does not supply the
verification feature, test-binary, doctest, or parity surface.

The GitHub workflow restores `~/.cache/nix`, while `NIXFIED_STATE_DIR` lives in
ephemeral runner storage. That does not preserve the MFM Cargo target and does
not by itself substitute MFM-specific Nix store outputs. Claims about CI reuse
must include the actual store, object cache, or runner lifecycle that supplies
the reused bytes.

## Architectural boundary under test

The following boundary is the provisional default and the baseline against
which alternatives compete.

| Authority | Responsibility |
| --- | --- |
| Nix | Pin and realize Rust, Cargo tools, native dependencies, immutable packages, and any explicitly selected immutable CI artifacts. |
| Cargo | Resolve and schedule the live Rust unit graph, fingerprints, features, profiles, build scripts, proc macros, rustdoc, linking, and developer incremental compilation. |
| Nixfied model | Declare project tasks, closures, services, dependencies, cache identities, and public verbs. |
| Nixfied runtime | Execute tasks, own mutable state and services, enforce placement and lifecycle, and retain run evidence. |
| CI provider | Supply ephemeral or persistent compute and the explicitly selected trusted artifact transport. |

The practical lanes are:

| Lane | Entry point | Source | Artifact policy | Purpose |
| --- | --- | --- | --- | --- |
| Focused development | persistent `nix develop` session, optionally activated by an environment tool, then direct Cargo | live worktree | incremental, full developer diagnostics, worktree-local target | edit/check/test feedback |
| Broad local verification | `nix run .#check`, `.#test`, `.#test-db`, or `.#ci` | live worktree | compact, nonincremental, Nixfied-scoped target | comprehensive evidence and managed services |
| CI verification | `nix run .#ci` | clean checkout | candidate selected by this RFC | merge evidence on Linux and macOS |
| Packaging | `nix build .#mfm` or `nix run .#mfm` | immutable Nix source | Nix derivation output | distributable CLI |

One public experience does not require one artifact cache. The unification
that matters is the pinned toolchain, declared task graph, feature/test
contracts, and evidence. Developer incremental state and durable verification
state have different invalidation, concurrency, retention, and diagnostic
requirements and should not be forced into one writable tree.

## Questions this RFC must answer

Revision 2 is complete only when it can answer all of these with measurements:

1. Does the two-lane artifact model remain the best developer and verification
   boundary, or does a supported Cargo mechanism safely improve cross-worktree
   reuse?
2. Is cold CI compilation material enough to justify persistent runners,
   compiler-object caching, or immutable Nix artifacts after transfer and
   lifecycle costs?
3. Which part of full-gate latency is compilation, linking, rustdoc, test
   execution, service startup, service execution, or avoidable task ordering?
4. Can one successful `.#ci` run on an exact commit supersede separately run
   component gates without losing evidence?
5. If Nix builds verification artifacts, can Nixfied consume those immutable
   closures directly without realizing them for unrelated tasks or copying a
   Cargo target?
6. Can `crate2nix` or `cargo2nix` provide useful per-crate derivation reuse
   while preserving MFM's exact feature, host/target, build-script, proc-macro,
   test-binary, and non-Rust input semantics?
7. Which cache ownership, inspection, lease, retention, cleanup, and evidence
   capabilities actually belong in Nixfied after the long-term design is
   selected?

## Hard constraints

Every experiment and decision remains subject to
[the code-quality policy](docs/code-quality.md),
[the architecture taxonomy](docs/architecture.md), and
[the design contract](docs/design.md).

The following are non-negotiable:

- the complete test, doctest, trybuild, SQLx, metadata, database, CLI, REST,
  keystore, and parity surface remains present;
- exact test identifiers, features, and environment-sensitive checks are
  compared at the same source revision;
- online SQLx schema validation executes live and is not satisfied by a
  compiler-cache hit;
- tests execute on every authoritative gate; compiled-artifact reuse is not
  test-result reuse;
- no secret-bearing environment, database credential, mnemonic, key, or
  fixture enters a report or cache;
- developer and verification mutable targets are not shared;
- mutable Cargo targets are never copied from or written into `/nix/store`;
- cache writes from untrusted workflows are prohibited unless the backend has
  a reviewed isolation model that prevents poisoning trusted builds;
- cache identity includes platform, architecture, exact toolchain, profile,
  features, build policy, and every mechanism-specific correctness input;
- every persistent cache has one owner, bounded retention, inspection,
  concurrency semantics, and safe cleanup before adoption;
- every candidate has a cache-bypass or cacheless path; and
- missing framework support is reported and designed properly rather than
  replaced by shell deletion, target copying, path rewriting, or generated
  stand-in crates.

A tool-generated `Cargo.nix` or equivalent Nix graph is not a stand-in crate.
It is allowed in the granular experiments when generation is reproducible,
drift is detectable, and the graph refers to the real locked sources. The
experiments may use bounded declarative crate overrides, but every override and
regeneration step counts toward maintenance cost. They may not fabricate Rust
source, patch around an unsupported semantic, or grow into an MFM-owned Cargo
graph implementation.

## Candidate assessment before testing

### Current scoped Cargo target

This is the control candidate. It has the lowest complexity and already gives
fast reuse when the right artifacts remain available. Its limitations are
mutable state, source-path sensitivity, cold hosted runners, incomplete
Nixfied cache lifecycle, and storage multiplication across slots and
worktrees.

It remains a valid long-term winner. A technology is not required merely
because this RFC evaluates technologies.

### Persistent trusted runner

A persistent runner can retain the current Cargo target and Nix store without
archive creation or transfer. It tests the simplest explanation for the
remaining cold/warm gap: keep the cache that Cargo already understands on the
machine that will reuse it.

The experiment must include host maintenance, isolation between repositories
and worktrees, trusted-job admission, disk limits, cache-family cleanup,
concurrent writer behavior, Nix store garbage collection, and recovery after a
failed or canceled run. A fast warm run is insufficient if the runner becomes
an unbounded, privileged mutable host.

### `sccache`

`sccache` preserves Cargo as the build planner and reuses compatible `rustc`
requests across target directories and source snapshots. It is the smallest
candidate that can add cross-run compiler reuse without creating a second Rust
dependency graph.

Its official Rust support requires incremental compilation to be disabled and
does not cache crates whose compiler invocation links final artifacts, such as
binaries and proc macros. Filesystem-reading proc macros are a correctness
caveat. It also does not eliminate rustdoc, Cargo planning, linking, test
execution, or service work. The pilot must report end-to-end time, not only a
headline cache-hit rate.

The first test is a bounded local or persistent-runner cache. Remote CI
persistence is tested only if the local screening result qualifies. Online
SQLx preparation runs with the compiler wrapper disabled. A cache-bypass run
must produce the same inventory and results.

### Crane dependency and verification artifacts

Crane is the coarse-grained Nix-native builder in the experiment set. It
continues to invoke Cargo while providing a maintained dependency-artifact
pattern and final-artifact consumers. This is materially different from the
separately tested per-crate graph generators.

The pilot is one end-to-end chain:

1. a pinned Crane dependency build produces `cargoArtifacts`;
2. a final verification build explicitly consumes those `cargoArtifacts`;
3. the final output contains a real Nextest archive or equivalent declared
   test-binary bundle; and
4. an opt-in Nixfied task executes that immutable output directly.

There is no standalone “dependency artifact succeeded” milestone. If the final
build does not consume the dependency output, or Nixfied cannot execute the
result, the pilot fails. MFM will not maintain a source-stub generator. Crane's
documented internal dummy-source technique may be used by the pinned upstream
library, but it is accepted only as an implementation detail of an artifact
that a real downstream build consumes.

The source set must explicitly include every compile or test input required by
MFM, including manifests, Rust sources, build scripts, migrations, `.sqlx`
metadata, trybuild `.rs` and `.stderr` files, examples or fixtures used by
`include_str!`/`include_bytes!`, target configuration, and relevant generated
inputs. A generic Rust-only source filter is not sufficient.

A Nextest archive is not the entire gate. Doctests are outside Nextest, and
trybuild tests may require a relocatable workspace plus nested Cargo. The
experiment must prove whether trybuild works from the immutable bundle. If it
does not, the report must identify the residual runtime-Cargo lane instead of
claiming full replacement.

Fresh hosted CI receives Nix reuse only through a persistent store or a signed,
trusted binary cache. Nix evaluation, derivation build, NAR creation,
upload/download, substitution, closure size, and signing/retention operations
all count toward the result. The artifact must not be added to a common model
in a way that makes `.#quick`, model admission, or unrelated tasks realize the
large closure eagerly.

### Execution topology

Execution topology is a first-class candidate even though it is not a cache.
The current full graph serializes broad check, workspace tests, keystore parity,
and the database composite. Trybuild suites are intentionally serialized
because they launch nested Cargo under a shared target lock. Some database
leaves are ordered by evidence dependencies rather than CPU availability.

The experiment will distinguish:

- build contention from executable test time;
- useful Nextest concurrency from CPU and memory oversubscription;
- trybuild's nested compilation from its UI-case execution;
- service startup from parity test execution;
- true data/service dependencies from conservative task ordering; and
- execution partitioning gains from duplicated artifact transfer or setup.

No ordering edge is removed merely because two tasks appear independent. Any
parallel candidate must preserve service isolation, schema isolation, evidence
ordering, resource bounds, and deterministic failure diagnostics.

### Granular Nix crate graphs: `crate2nix` and `cargo2nix`

`crate2nix` and `cargo2nix` are viable experiment candidates for the specific
hypothesis that Nix should cache third-party dependencies and MFM workspace
crates as separate derivations. Both generate Nix expressions from Cargo
workspace and lock information and build crates in isolation. This could give
Nix exact per-crate reuse across worktrees and CI substitutions while Cargo
continues to own the developer loop.

They are tested as two independent candidates. Success by one is not evidence
for the other, and neither is combined with Crane or `sccache` during
screening. Each pilot must:

1. generate its graph deterministically from the real workspace and
   `Cargo.lock`;
2. use the exact pinned MFM Rust and native toolchain on Linux and macOS;
3. build real dependency and workspace crate derivations without generated
   Rust stand-ins;
4. produce final verification binaries or an immutable bundle that consumes
   those per-crate outputs;
5. let an opt-in Nixfied task execute the final artifacts directly; and
6. prove which derivations rebuild or substitute for every common source/cache
   matrix case.

Graph generation alone is not a passing result. A root package that builds but
cannot expose the real MFM test binaries is also insufficient. If a tool's test
support couples compilation and execution into a cacheable derivation, that
surface may be used to diagnose graph fidelity, but its cached success cannot
replace executing tests in the authoritative Nixfied gate.

The known hard cases are deliberate tests: command-specific feature
unification, target-specific dependencies, host versus target units, build
scripts, proc macros, native dependencies, cross-workspace `include_str!`
inputs, migrations and `.sqlx`, trybuild sources and stderr baselines,
doctests, and final binary linkage. Current `crate2nix` documentation marks its
test interface experimental and documents target-feature and workspace-source
restrictions. Current `cargo2nix` documents global feature choices, propagated
link/build information, generated-graph upkeep, and package overrides. The
pilot measures whether these constraints are manageable for MFM rather than
assuming either success or failure.

The comparison includes graph generation mode, generated-file drift,
evaluation time, derivation count, rebuild fan-out, output and closure size,
binary-cache transfer, override volume, upgrade workflow, and diagnostics. A
new MFM-specific graph generator remains rejected: maintained upstream tools
are being tested precisely so MFM does not invent a third implementation.

### Cargo `build-dir` evolution

Cargo now permits separating intermediate `build-dir` from final `target-dir`,
and the MFM toolchain accepted a shared `CARGO_BUILD_BUILD_DIR` in disposable
probes. One focused package probe produced these exploratory results:

| Probe | Observation |
| --- | ---: |
| Seed build into a fresh shared build directory | 3.23s |
| Different target directory, same shared build directory | 0.07s |
| Different target directory, fresh build directory control | 2.94s |
| Identical workspace copied to another path, same build directory | 0.08s |
| Comment edit in the copied leaf crate | 0.45s |
| Original and edited copies after both variants existed | 0.08s each |

These results are promising but not adoption evidence. The Cargo team does not
currently endorse sharing one build directory across workspaces. The new
content-scoped layout, granular locking, cross-workspace caching, and automatic
stale-unit cleanup are still evolving. Revision 2 records a bounded
compatibility probe but will not make unsupported cross-workspace sharing an
authoritative MFM path. Cargo-native support should replace a custom cache when
its ownership and compatibility contract becomes official.

## Common experiment contract

### Canonical source and cache matrix

Every compilation candidate is tested against the same cases:

1. clean environment with no MFM artifact cache;
2. exact unchanged rerun;
3. documentation-only tracked-file change;
4. edit to a leaf workspace crate;
5. edit to a widely shared crate such as `mfm-ids` or `mfm-canonical`;
6. change to a manifest or `Cargo.lock`;
7. feature or verification-profile change;
8. build-script, migration, `.sqlx`, included-file, or trybuild-baseline change;
9. identical source in a second worktree path;
10. concurrent worktrees or slots when the candidate permits concurrency;
11. exact toolchain or target-platform change; and
12. explicit cache bypass followed by normal reuse.

Source edits use disposable worktrees or generated patches with recorded
digests. They are restored before repository gates run. No experiment mutates
an active developer or authoritative verification cache merely to manufacture
a clean state.

### Measurements

Each run records:

- source commit, dirty-state fingerprint, worktree identity, and experiment
  configuration;
- operating system, architecture, runner class, CPU, memory, filesystem, and
  relevant host load;
- exact Nix, Nixpkgs, Nixfied, Rust, Cargo, Nextest, SQLx, Crane, `crate2nix`,
  `cargo2nix`, and `sccache` identities as applicable;
- model hash, task graph, command, features, profiles, and cache state;
- Nix evaluation, realization, build, substitution, and transfer time;
- Cargo planning, compilation, linking, rustdoc, and nested-Cargo time where
  observable;
- test execution, service startup, parity execution, and total wall time;
- exact Nextest identifiers and hashes plus doctest, trybuild, SQLx, metadata,
  database, CLI, REST, and keystore inventories;
- Cargo target, compiler cache, Nix output and closure sizes, NAR sizes, file
  counts, and repeated-run growth;
- generated-graph digest and drift, Nix evaluation time, derivation count, and
  derivations rebuilt, reused locally, or substituted for granular candidates;
- hit, miss, cacheability, eviction, upload, download, and bypass statistics;
  and
- failure diagnostics, backtrace file/line quality, cleanup behavior, and
  recovery after cancellation.

Task durations that overlap are not summed and presented as wall time. A
compiler hit rate is not presented as end-to-end improvement. A local Nix
store hit is not presented as fresh-CI reuse. Every report distinguishes facts,
inferences, and unverified surfaces.

### Screening and qualification

Experiments use two stages so obviously poor candidates do not consume a full
cross-platform matrix.

Screening uses isolated Linux runs with at least two clean and three warm
samples, plus exact-source, leaf-edit, shared-crate-edit, and bypass cases. A
candidate stops at screening when it:

- changes coverage or results;
- has an unexplained cache hit after a compile-relevant input change;
- cannot be consumed by the workload it claims to accelerate;
- cannot define safe ownership, trust, or cleanup;
- increases median end-to-end time; or
- saves less time than its measured lookup, realization, or transfer overhead
  with no compensating correctness or operational benefit.

Qualification uses at least three clean and five warm samples on the actual
target environments, including hosted or persistent Linux and macOS as
applicable. It runs the complete source/cache matrix and reports distributions,
not only the median. Platform-specific strategies are allowed, but an
unmeasured platform is not silently covered by another platform's result.

### Correctness vetoes

Any of the following rejects a candidate regardless of speed:

- missing or changed required test identifiers, features, doctests, trybuild
  cases, SQLx checks, parity leaves, or output contracts;
- stale success after a compile-relevant source, schema, toolchain, target,
  feature, or profile change;
- secret persistence or an untrusted cache write reaching trusted builds;
- mutation of Nix store outputs or copying a mutable target as a cache format;
- unsafe cleanup, path traversal, symlink following, state-family confusion,
  or deletion of service/run evidence;
- concurrent corruption or an undefined same-cache writer policy;
- diagnostics that no longer identify actionable source files and lines; or
- a hidden fallback that rebuilds through a different toolchain or undeclared
  host dependency.

### Materiality and complexity

A cache or runner candidate qualifies for architectural consideration only if
it improves the median end-to-end target workload by both at least 15% and at
least 30 seconds, or if it supplies a separately approved reproducibility or
operational property that the baseline cannot provide. It must not introduce a
repeatable regression greater than 10% in comparable cacheless, changed-source,
or warm workloads.

Those thresholds are gates for consideration, not an automatic selection.
The decision also weighs:

- maintenance and upgrade burden;
- number of artifact authorities and invalidation models;
- storage and network cost;
- trusted infrastructure requirements;
- failure and rollback behavior;
- visibility into cache use and misses;
- performance with no cache; and
- whether execution, rather than compilation, is now the dominant residual.

The target-only baseline wins ties. A combination is considered only when each
component qualifies independently and an isolated combined experiment proves
material additive value. Using different single mechanisms in different
environments is not considered a hybrid when each invocation has one clear
artifact authority.

## Revision 2 phase plan

Revision 2 phase identifiers use the `R2-` prefix so they cannot be confused
with the completed original phases recorded in the baseline report.

Each phase is separately approved. Experimental code lives in a disposable
worktree or explicitly temporary experiment branch unless the phase says
otherwise. A report-only conclusion is a valid success. No pilot dependency,
task, workflow, cache directory, generated source, or flake input remains in
the main branch merely to preserve a failed experiment.

### R2-00: accept the reset

Change:

- replace the old forward plan with this RFC;
- preserve the original Phase 00–07 evidence and Phase 08 rejection; and
- make no build, workflow, cache, test, or runtime behavior change.

Evaluate:

- local links and Markdown structure;
- consistency with the current flake, Nixfied model, CI workflow, and baseline;
- explicit separation of experiments from decisions; and
- repository-required documentation verification.

Expected commit subject:

```text
docs: reset slow-build architecture evaluation
```

Stop decision: approve or revise the experiment contract before running a new
candidate.

### R2-01: refresh the workload and gate baseline

Change:

- extend the baseline report with current local Linux, hosted Linux, and hosted
  macOS observations at one exact revision;
- add no cache technology; and
- record the admitted task graph and public-gate equivalence.

Test:

- focused Cargo check/test from a persistent Nix development environment;
- `.#quick`, each public Nixfied gate, `.#ci`, and `nix flake check`;
- clean and warm compilation versus execution/service decomposition;
- exact test, feature, doctest, trybuild, SQLx, metadata, and parity inventory;
- whether `nix flake check` currently performs Rust compilation;
- whether `.#ci` contains the same `check`, `test`, and `test-db` leaves and
  semantics exposed by the component verbs; and
- the real contribution-workflow cost of running component gates and then
  rerunning `.#ci` on the same source revision.

Expected commit subject:

```text
docs: refresh build workload baseline
```

Stop decision: freeze the canonical workloads and determine whether gate
governance is eligible for a later policy change. Do not change `AGENTS.md` in
this phase.

### R2-02: test the developer build boundary

Change:

- add only reportable, disposable probes; and
- keep direct Cargo, `.#quick`, and all public gates unchanged.

Test:

- persistent `nix develop` plus focused Cargo against invoking `.#quick` for
  each edit cycle;
- warm leaf, reverse-dependent, and broad-check feedback;
- independent worktree-local target behavior and disk growth;
- stable `build-dir` separation with distinct target directories;
- identical-source relocation, one-leaf edit, concurrent access, trybuild,
  rust-analyzer coexistence, and cleanup; and
- the nightly new build layout only as an upstream compatibility observation.

Unsupported cross-workspace build-directory sharing cannot win this phase. The
result is either confirmation of the current developer boundary or a list of
upstream Cargo conditions that would justify retesting it later.

Expected commit subject:

```text
docs: record developer build boundary experiment
```

Stop decision: define the developer-lane input to the final architecture
comparison.

### R2-03: test verification execution topology

Change:

- create opt-in or disposable task-graph variants only; and
- make no ordering or partitioning change authoritative.

Test:

- Nextest worker counts and, where useful, partitions against a precompiled or
  fully warm test set;
- serialized trybuild versus safe alternatives that do not contend on one
  Cargo lock;
- doctest cost and whether it overlaps safely with binary test execution;
- Postgres startup, migration, schema isolation, and each parity leaf's true
  dependency/resource requirements;
- current serial composite ordering against bounded parallel candidates;
- CPU, memory, disk, Cargo-lock, service-port, and failure-diagnostic behavior;
  and
- total wall time without counting duplicated compilation or setup as an
  execution improvement.

Any qualifying topology change becomes a separately scoped implementation
proposal. This experiment does not weaken or cache test execution.

Expected commit subject:

```text
docs: record verification execution experiment
```

Stop decision: identify execution improvements worth carrying into the
candidate comparison and record the remaining compilation ceiling.

### R2-04: test persistent Cargo verification

Change:

- run the current compact, scoped Cargo verification target on a disposable
  persistent trusted runner or equivalent isolated long-lived host; and
- add no compiler wrapper or Nix-built verification artifact.

Test:

- the complete common source/cache matrix;
- unchanged, leaf-edit, shared-crate-edit, and cross-worktree reuse;
- trusted-job admission and repository/worktree isolation;
- same-slot and cross-slot concurrency;
- Cargo target and Nix store size, repeated growth, retention, garbage
  collection, inspection, and cancellation recovery;
- the effect of the pinned Nixfied runtime's missing cache-family lifecycle;
  and
- total ownership and maintenance cost compared with ephemeral hosted runners.

Expected commit subject:

```text
docs: record persistent cargo verification experiment
```

Stop decision: qualify the runner/target baseline, reject it, or identify a
framework capability that must be included in the final handoff.

### R2-05: test bounded `sccache`

Change:

- add a pinned, opt-in compiler wrapper only in an isolated experiment;
- retain the compact nonincremental verification profile;
- bound the cache and expose a bypass; and
- disable the wrapper for online SQLx preparation.

Test:

- the common source/cache matrix and exact coverage inventory;
- local exact-hit, second-worktree, leaf-edit, and shared-crate hit rates;
- cacheable versus non-cacheable compiler requests, proc macros, binaries,
  linking, rustdoc, trybuild, and residual execution;
- source-path normalization and false-hit probes for filesystem-reading macros
  and included files;
- cache size, evictions, lookup overhead, corruption behavior, and cleanup; and
- byte-for-byte or semantically equivalent results under cache bypass.

Only after local screening qualifies may the phase test trusted CI persistence.
That extension includes archive or remote transfer time, read/write trust,
poisoning boundaries, retention, Linux/macOS separation, and failure when the
cache backend is unavailable.

Expected commit subject:

```text
docs: record bounded sccache experiment
```

Stop decision: qualify or reject `sccache` as a standalone environment-specific
candidate. Do not combine it with any Nix-native compilation candidate in this
phase.

### R2-06: test a consumed Crane verification artifact

Change:

- pin Crane only in an isolated pilot;
- build a real dependency artifact and a final consumer in one experiment;
- produce a Nextest archive or equivalent immutable verification bundle;
- expose an opt-in Nixfied execution path only if the model can consume the
  closure directly; and
- leave public gates authoritative and unchanged.

Test:

- proof from derivation inputs/logs that the final artifact consumed the
  dependency output;
- the complete common source/cache matrix, including explicit non-Rust inputs;
- exact test-binary and test-identifier parity with Cargo verification;
- execution from an immutable output without target copying or mutation;
- relocation and nested-Cargo behavior of all nine trybuild harnesses;
- the residual Cargo lane for doctests, trybuild if necessary, and online SQLx;
- Nixfied model admission and unrelated-task realization cost;
- local-store exact hits and changed-source rebuild boundaries;
- output, closure, NAR, evaluation, build, substitution, and garbage-collection
  sizes/costs; and
- signed binary-cache transfer and trust on fresh Linux/macOS runners only
  after local screening qualifies.

The pilot fails immediately if it generates repository-owned stand-in crate
sources, leaves the dependency output unconsumed, copies a mutable target, or
cannot execute the final artifact. Missing Nixfied task-specific closure
support is a reported capability gap, not permission for a shell workaround.

Expected commit subject:

```text
docs: record consumed nix artifact experiment
```

Stop decision: qualify or reject coarse Nix-native verification and record the
baseline against which granular Nix reuse will be compared.

### R2-07: test granular `crate2nix` artifacts

Change:

- pin `crate2nix` in an isolated pilot;
- generate its Nix graph from the exact MFM workspace and lockfile;
- build real per-dependency and per-workspace-crate derivations;
- attempt to produce and directly consume a final immutable verification
  artifact from that graph; and
- leave public gates, the main flake inputs, and the authoritative Cargo path
  unchanged.

Test:

- deterministic `Cargo.nix` generation, committed versus IFD generation cost,
  and an explicit stale-generated-graph failure check;
- Cargo.lock package/version equality and the active feature, target,
  host/build, build-script, proc-macro, native-link, and profile surfaces;
- the complete common source/cache matrix and exact derivations rebuilt,
  reused locally, or substituted in every case;
- cross-workspace included files, migrations, `.sqlx` metadata, trybuild
  sources and stderr files, examples, and generated inputs;
- documented target-feature and workspace-source restrictions against MFM's
  actual graph;
- final test-binary and test-identifier parity, including whether artifacts can
  be exposed for repeated Nixfied execution rather than only caching a
  `runTests` derivation's success;
- doctest, trybuild nested-Cargo, online SQLx, and live-service residuals;
- direct Nixfied closure consumption without mutable-target copying or eager
  realization by unrelated tasks;
- graph generation/evaluation time, derivation count, rebuild fan-out, local
  build time, output/closure/NAR size, and signed binary-cache transfer; and
- crate overrides, unsupported cases, diagnostic quality, regeneration
  workflow, and expected maintenance across crate2nix, Cargo, Rust, and
  Nixpkgs upgrades.

The pilot fails screening if it can only build a convenient subset that omits
MFM's verification surface, requires fabricated Rust sources or semantic
patches to imitate Cargo, or produces outputs that no verification task
consumes. Small declarative native-dependency or platform overrides are
permitted but must be listed and costed.

Expected commit subject:

```text
docs: record crate2nix artifact experiment
```

Stop decision: qualify or reject `crate2nix` independently. Do not alter the
`cargo2nix` experiment or create an MFM-specific graph generator.

### R2-08: test granular `cargo2nix` artifacts

Change:

- pin `cargo2nix` in an isolated pilot;
- generate its `Cargo.nix` from the same exact MFM workspace and lockfile used
  by R2-07;
- construct the per-crate package set with the MFM-pinned Rust toolchain;
- attempt to produce and directly consume a final immutable verification
  artifact from that graph; and
- leave public gates, the main flake inputs, and the authoritative Cargo path
  unchanged.

Test:

- deterministic graph generation, generated-file drift, and generator/overlay
  version compatibility;
- Cargo.lock package/version equality and the active root-feature, global
  feature, target, host/build, build-script, proc-macro, native-link, and
  profile surfaces;
- the complete common source/cache matrix and exact derivations rebuilt,
  reused locally, or substituted in every case;
- propagated rlib, linker, build-script, and native dependency information;
- all cross-crate included files, migrations, `.sqlx` metadata, trybuild
  sources and stderr files, examples, and generated inputs;
- final test-binary and test-identifier parity and repeated Nixfied execution
  independent of any cached build-time test result;
- doctest, trybuild nested-Cargo, online SQLx, and live-service residuals;
- direct Nixfied closure consumption without mutable-target copying or eager
  realization by unrelated tasks;
- graph generation/evaluation time, derivation count, rebuild fan-out, local
  build time, output/closure/NAR size, and signed binary-cache transfer; and
- package overrides, unsupported cases, diagnostic quality, regeneration
  workflow, and expected maintenance across cargo2nix, Cargo, Rust, rust-overlay,
  and Nixpkgs upgrades.

The same screening failure and bounded-override rules from R2-07 apply. The
result is assessed on its own; it neither inherits a crate2nix failure nor wins
because crate2nix failed.

Expected commit subject:

```text
docs: record cargo2nix artifact experiment
```

Stop decision: qualify or reject `cargo2nix` independently. Do not create an
MFM-specific graph generator to rescue the result.

### R2-09: compare qualifying candidates

Change:

- rerun only candidates that passed screening under the qualification matrix;
- normalize reports into one comparison; and
- introduce no default architecture.

Test:

- three clean and five warm runs on each target environment;
- identical source/cache, coverage, trust, storage, and failure cases;
- end-to-end local, persistent-runner, and fresh-hosted behavior as applicable;
- residual compilation, linking, rustdoc, execution, and service costs;
- cacheless and backend-unavailable behavior; and
- any combined candidate only after separate owner approval and only when both
  standalone components qualified with a specific additive hypothesis.

The report fills this decision table with measured values:

| Candidate/environment | Full-gate distribution | Leaf/shared edit | Transfer/realization | Persistent bytes | Coverage | Operations | Verdict |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Scoped Cargo target | pending | pending | none | pending | pending | pending | pending |
| Persistent runner | pending | pending | none | pending | pending | pending | pending |
| `sccache` | pending | pending | pending | pending | pending | pending | pending |
| Crane artifact | pending | pending | pending | pending | pending | pending | pending |
| `crate2nix` artifacts | pending | pending | pending | pending | pending | pending | pending |
| `cargo2nix` artifacts | pending | pending | pending | pending | pending | pending | pending |
| Execution topology | pending | n/a | n/a | pending | pending | pending | pending |

Expected commit subject:

```text
docs: compare qualifying build candidates
```

Stop decision: declare the evidence decision-quality or request one narrowly
defined missing measurement. Do not select by preference or prototype effort
already spent.

### R2-10: select and define the long-term architecture

Change:

- update this RFC and add an ADR if appropriate;
- select the least complex qualifying strategy for development, broad local
  verification, hosted or persistent CI, and packaging;
- define artifact authority, identities, inputs, ownership, trust, retention,
  inspection, concurrency, cleanup, evidence, failure, and rollback;
- decide whether execution-topology changes proceed in a separate
  implementation plan; and
- decide gate governance, including whether one successful `.#ci` on an exact
  candidate supersedes separate `.#check`, `.#test`, and `.#test-db` runs.

The decision may keep the provisional Cargo/Nix/Nixfied boundary, add one
environment-specific cache, use immutable CI artifacts, or select no new
compilation cache. It must explicitly state what was rejected and why.

This phase defines rollout prerequisites but does not silently enable the
winner. Implementation and rollout require a new, small, reviewable plan after
the architecture and any upstream dependencies are accepted.

Expected commit subject:

```text
docs: select mfm build architecture
```

Stop decision: owner approval of the complete long-term definition and the
minimal upstream capabilities it requires.

### R2-11: finalize the Nixfied architect handoff

This is the final phase of this RFC and cannot begin before R2-10 is accepted.

Change:

- rewrite [the Nixfied capability-gap document](docs/nixfied-capability-gaps.md)
  from the selected architecture rather than the provisional Phase 06 design;
- remove requests that belong only to rejected candidates;
- separate capabilities required for rollout from optional future improvements;
- include minimal reproductions, exact pinned revisions/ABIs, security and
  lifecycle invariants, and framework-level acceptance tests; and
- map each request to the experiment and decision evidence that justifies it.

The handoff must answer, where applicable:

- whether Nixfied owns Cargo cache-family inspection, leases, retention, and
  garbage collection;
- whether task-specific immutable closures can avoid eager realization by
  unrelated tasks;
- how a task reports the immutable or mutable artifact identity it consumed;
- how cache cleanup preserves service state, run records, logs, and evidence;
- how worktree, slot, process, and concurrent-writer identity are enforced; and
- which behavior MFM will own locally instead of requesting from Nixfied.

Evaluate:

- every request has a selected-architecture consumer;
- every P0 has a failing MFM scenario and a framework-level acceptance test;
- no request asks Nixfied to reproduce Cargo's unit graph or become a Rust
  compiler cache;
- the document is understandable without the abandoned Phase 08 design; and
- MFM's owner approves the document before it is sent to Nixfied's architects.

Expected commit subject:

```text
docs: finalize nixfied capability handoff
```

Stop decision: send the reviewed document to Nixfied's architects, then create
a separate implementation plan once required framework decisions are known.

## Phase governance

Approval to rewrite this RFC authorizes only R2-00. Each later phase requires
explicit approval and stops after its report. One phase should produce at most
one logical main-branch commit; temporary experiment branches may contain the
minimum commits needed to obtain CI evidence and are removed after the report.

For every phase:

1. confirm the exact scope, clean base commit, and absence of overlapping
   user-owned work;
2. record the host, toolchain, source, task graph, cache state, and baseline
   before changing an experiment;
3. use focused Cargo checks while developing and isolate destructive or
   cache-state probes;
4. preserve the exact coverage inventory and run all repository-required gates
   before any main-branch commit;
5. run `nix run .#ci` for major, final, task, cache, Nix, workflow, or
   architecture changes under the currently authoritative repository policy;
6. evaluate the clean committed report tree and distinguish dirty-worktree
   measurements from exact derivation/cache results;
7. remove temporary pilot surfaces that are not part of the report; and
8. deliver a self-contained stop report with recommendation and unverified
   surfaces.

A failed experiment is not repaired with a narrower claim after the fact. It
is reported as failed or mixed, with the exact residual surface. A missing
framework feature is a blocker for that candidate, not permission to add a
project-local approximation.

## Required phase report

Every phase report uses this minimum shape:

```text
Phase:
Commit: <hash and lower-case subject, or not committed>
Outcome: passed | failed | mixed | blocked

Changed:
- durable report/config changes
- temporary experiment surfaces and their cleanup
- explicit non-changes

Verification:
- focused checks
- required gates and run identifiers
- exact coverage and feature inventory comparison

Measurements:
- source/cache cases and sample distributions
- compile/link/rustdoc versus execution/service time
- target/cache/store/closure/NAR/file statistics
- transfer, hit/miss, cleanup, and failure behavior

Acceptance:
- correctness vetoes
- screening or qualification thresholds
- operational and platform requirements

Risks and limitations:
- variance, missing environments, unsupported behavior, and uncertainty

Recommendation:
- proceed | hold | reject | redesign
- exact scope requested next
```

Reports contain no credentials, secrets, private keys, mnemonics, or
secret-bearing paths or environment values.

## Explicitly rejected shortcuts

The following are outside Revision 2 without a new RFC:

- one writable Cargo target shared by developer, verification, CI, and sibling
  worktrees;
- a repository-owned generator that fabricates dependency crate sources;
- a Nix derivation whose output is not consumed by the measured workload;
- copying a prewarmed Cargo target from `/nix/store` into mutable state;
- adopting `crate2nix` or `cargo2nix` without the R2-07/R2-08 graph-fidelity,
  artifact-consumption, and full-coverage evidence;
- an MFM-specific replacement for Cargo's unit graph;
- calling a Nix launcher or vendored source derivation a compiled Rust cache;
- persisting the entire mutable Cargo target through a generic CI archive;
- enabling `sccache` in the incremental developer lane or online SQLx check;
- combining Crane and `sccache` before either qualifies independently;
- treating local Nix store hits as fresh hosted-CI results;
- caching successful test outcomes;
- removing trybuild, doctest, SQLx, parity, metadata, CLI, REST, or keystore
  coverage to meet a latency target;
- merging architectural crates to reduce package count;
- broad Nixfied slot deletion or shell-level recursive cleanup presented as
  cache-family lifecycle; and
- changing gate policy before proving exact task/evidence equivalence.

## Likely outcomes, without preselecting one

The evidence may support different mechanisms at different boundaries:

- developers may keep Nix-provided tools plus worktree-local incremental Cargo;
- broad local verification may keep the compact Nixfied Cargo target;
- trusted persistent CI may reuse that target without transfer;
- ephemeral CI may use `sccache` or a signed Nix verification artifact if one
  qualifies after transfer costs;
- a qualifying `crate2nix` or `cargo2nix` graph may provide per-crate Nix cache
  reuse for verification while developers continue to use Cargo directly;
- packaging may remain a coarse `buildRustPackage` derivation; and
- full-gate latency may improve more from execution topology and gate
  governance than from another compile cache.

That is still one coherent platform if every boundary has one documented
artifact authority and the same source, toolchain, coverage, and evidence
contract. Uniformity of mechanism is less important than clarity of ownership.

## External technical references

- [Cargo build cache](https://doc.rust-lang.org/cargo/reference/build-cache.html)
  and [`build.build-dir` configuration](https://doc.rust-lang.org/cargo/reference/config.html#buildbuild-dir)
- [Cargo unstable unit graph](https://doc.rust-lang.org/cargo/reference/unstable.html#unit-graph)
- [Cargo build-directory layout v2 and cross-workspace-cache direction](https://blog.rust-lang.org/2026/03/13/call-for-testing-build-dir-layout-v2/)
- [Crane artifact reuse](https://crane.dev/introduction/artifact-reuse.html)
  and [API](https://crane.dev/API.html)
- [Nextest archived builds](https://nexte.st/docs/ci-features/archiving/)
  and [test-runner scope](https://nexte.st/)
- [`sccache` Rust limitations](https://github.com/mozilla/sccache/blob/main/docs/Rust.md)
- [Nix signed binary-cache configuration](https://nix.dev/guides/recipes/add-binary-cache.html)
- [crate2nix crate-by-crate design](https://nix-community.github.io/crate2nix/),
  [test support](https://nix-community.github.io/crate2nix/30_building/40_tests/),
  and [known restrictions](https://nix-community.github.io/crate2nix/90_reference/20_known_restrictions/)
- [cargo2nix granular-build design](https://github.com/cargo2nix/cargo2nix)
- [Pinned Nixfied architecture](https://github.com/willyrgf/nixfied/blob/b0681e45ab76d5023d9c5e033087d34adf98e90b/docs/ARCHITECTURE.md)
