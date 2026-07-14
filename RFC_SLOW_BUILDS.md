# RFC: faster Rust builds and verification

Status: accepted near-term remediation; long-term cache architecture under evaluation

Date: 2026-07-14

## Summary

MFM's compile and verification workflow is slow enough to interrupt normal
development, and its persistent build artifacts have grown disproportionately
large. The problem is not one unusually slow crate. It is the interaction of:

- comprehensive workspace-wide verification;
- duplicated test execution;
- avoidable artifact invalidation;
- one long-lived Nixfied Cargo target serving many build shapes and checkouts;
- incremental compilation and unpacked split debug information in verification
  builds;
- expensive test, doctest, compile-fail, and live-service execution.

MFM will address this with two intentionally different build lanes:

1. a fast, incremental developer lane for focused feedback; and
2. a comprehensive, disk-efficient verification lane for the existing Nixfied
   gates and CI.

The first implementation work will remove demonstrably duplicated work and the
redundant SQLx package clean. The verification lane will then receive a
centrally enforced artifact policy and a purpose-specific, bounded Cargo target
cache. MFM will next evaluate Nix-native compiled artifacts before evaluating
compiler-object caching with `sccache`. The long-term platform design will be
selected only after the improved Cargo baseline, Nix-native artifacts,
`sccache`, and any justified hybrid have been measured under the same
workloads.

This RFC does not weaken the test matrix, merge architectural crates, change
public behavior, or replace full verification with the developer lane.

### Decisions and hypotheses

The following decisions are accepted independently of the long-term cache
mechanism:

- remove confirmed duplicate test execution;
- remove the redundant SQLx package clean;
- separate focused developer feedback from comprehensive verification;
- give verification an explicit, disk-efficient artifact policy;
- isolate mutable Cargo targets by purpose and worktree/slot;
- preserve the complete assurance matrix; and
- require controlled, comparable measurements before adopting an optimization.

The long-term verification cache remains a hypothesis to test. The candidate
designs are:

1. the improved mutable Cargo target without an additional compilation cache;
2. immutable Nix derivations for dependency and verification artifacts;
3. compiler-unit reuse through `sccache`; and
4. a hybrid, but only if its measured benefit justifies operating both systems.

| Candidate | Reuse unit | Expected strength | Primary limitation |
| --- | --- | --- | --- |
| Improved Cargo target | Cargo fingerprints and incremental units | Lowest complexity and fastest warm edit loop | Mutable, path-sensitive, and difficult to share safely |
| Nix-native artifacts | Immutable declared derivation outputs | Exact cross-worktree reuse with reproducible ownership | Rebuild quality depends on derivation/source granularity |
| `sccache` | Individual compatible `rustc` requests | Reuse across different Cargo invocations and source snapshots | Does not cover linking, rustdoc, execution, or ambient inputs |
| Hybrid | Nix artifacts plus compiler-object reuse where non-overlapping | Potentially covers exact and compiler-unit reuse | Two invalidation, storage, trust, and failure models |

No candidate is selected by this RFC before the evaluation completes.

## Problem situation

The MFM workspace contains many deliberately small crates with strong typed
boundaries. Its verification matrix includes workspace compilation, Clippy,
Nextest, doctests, trybuild UI contracts, Cargo metadata contracts, SQLx schema
checks, and live-service parity tests. Those checks protect architecture and
runtime behavior and are not accidental overhead.

The current workflow nevertheless combines comprehensive assurance with build
artifact policies better suited to interactive development. It also performs
some work twice and explicitly invalidates artifacts immediately before later
tasks need related build units. As a result:

- broad gates are used where focused feedback is sufficient during iteration;
- verification stores large amounts of incremental and split-debug state;
- source-path, feature, profile, and checkout changes accumulate in a shared
  target tree;
- clean and partially invalidated runs vary substantially in duration;
- direct Cargo and Nixfied builds cannot safely reuse each other's raw target
  directories;
- compile-time improvements alone cannot eliminate the substantial time spent
  executing tests and parity checks.

The required solution is therefore a build-system and task-graph design, not a
single compiler flag or dependency cleanup.

## Evidence and interpretation

The investigation used the current `mfm3` checkout, workspace configuration in
[Cargo.toml](Cargo.toml), Cargo metadata and tree inspection, task definitions
in [nixfied.nix](nixfied.nix), persisted Nixfied run records, and the existing
build artifacts.

### Workspace and artifact observations

| Measurement | Observation |
| --- | --- |
| Workspace | 53 packages and 100 targets |
| Fully resolved metadata universe | 504 packages and 1,808 dependency edges |
| Active host graph | Approximately 348 packages on the inspected Linux feature/target surface |
| Active build machinery | Approximately 30 build-script packages and 18 proc-macro crates on that surface |
| Local Cargo target | Approximately 11 GiB in `target/` |
| Nixfied Cargo target | Approximately 44 GiB in persistent Nixfied state |
| Nixfied incremental artifacts | Approximately 21 GiB |
| Nixfied dependency artifacts | Approximately 19 GiB |
| Nixfied trybuild artifacts | Approximately 4 GiB |
| Split-debug artifacts | Approximately 190,000 `.dwo` files occupying about 12 GiB |
| Warm workspace check | `cargo check --workspace --all-targets` completed in 1.77 seconds |

The 504-package metadata result is not the graph compiled by every host build.
It includes inactive platform packages, optional feature surfaces, and multiple
versions selected by different dependency branches. Dependency cleanup remains
worthwhile when verified, but the raw resolved-package count must not be used as
the primary optimization target.

The 1.77-second warm check is also not a clean-build benchmark. It demonstrates
that Cargo is already fast when the correct artifacts exist. The slow path is
dominated by missing, fragmented, or invalidated artifacts plus real test and
service execution.

### Timing evidence and its limitations

The original investigation selected a persisted 440.6-second Nixfied run. That
run contained a parity task that has since been removed from the current task
graph, so it is useful diagnostic evidence but not a valid current baseline.

More recent full runs using the current task graph ranged from approximately
230 to 643 seconds depending on cache and invalidation state. This variance is
itself evidence of an unstable artifact strategy. It also means that an
uncontrolled single run must not be used to accept or reject an optimization.

In the selected 440.6-second run:

- workspace Nextest took 112.6 seconds, including approximately 77.3 seconds
  of test execution;
- workspace doctests took 130.1 seconds;
- the separate `mfm-app` test-support task took 27.7 seconds;
- a parity report task spent approximately 56 seconds executing its test.

In a later 230.6-second current-graph run:

- workspace Nextest took 76.7 seconds;
- the separate `mfm-app` test-support task took 19.9 seconds;
- workspace doctests took 16.5 seconds;
- one parity report task took 66.8 seconds.

The difference between the runs shows the importance of controlling cache
state. The execution portions also establish a ceiling for changes that affect
only compilation.

### Confirmed duplicated test execution

The workspace Nextest invocation already includes the `mfm-app` tests compiled
with `test-support`. The `mfm-integration-tests` package enables
`mfm-app/test-support` as a normal dependency in
[tests/integration/Cargo.toml](tests/integration/Cargo.toml), so workspace
feature unification makes those tests available to the workspace run.

The test identifiers from the separate `mfm-app` invocation exactly matched
the corresponding identifiers in workspace Nextest in the inspected runs: 70
duplicated tests in the original run and 44 in a later run after the test graph
changed. This is duplicated execution, not additional assurance.

### Confirmed SQLx invalidation

The online SQLx task in [nixfied.nix](nixfied.nix) explicitly runs:

```text
cargo clean -p mfm-stream-store-postgres
```

before `cargo sqlx prepare --check`. The pinned SQLx CLI already performs a
minimal-project recompilation setup: it touches the relevant package target
sources and has its own cleaning fallback when that setup fails. MFM's
preceding package clean is therefore redundant and removes reusable outputs for
all profiles and feature shapes of the package.

A separate target directory is not the initial solution. SQLx touches source
mtimes in the shared checkout, which can still make the ordinary verification
target stale, while a second target forces another compilation. The correct
first step is to remove the explicit clean, keep SQLx in the verification
target, and measure the resulting invalidation.

### Current profile and cache behavior

The workspace development and test profiles in [Cargo.toml](Cargo.toml) use
unpacked split debug information. The existing `[profile.ci]` inherits the test
profile and sets `codegen-units = 256`, which is effectively the normal
development/test default in the current incremental configuration. Nixfied
tasks do not select that profile consistently, so it does not currently define
a verification artifact policy.

Nixfied sets one target directory for its Cargo leaves:

```text
CARGO_TARGET_DIR=${stateDir}/cargo-target
```

The project id and state slot can be shared by sibling checkouts. Raw Cargo
target trees contain absolute source paths, fingerprints, profiles, feature
sets, incremental state, and locks. Sharing one mutable tree across direct
Cargo, Nixfied, CI, or concurrent worktrees produces unreliable reuse and
unbounded accumulation.

The [GitHub checks workflow](.github/workflows/checks.yml) currently preserves
the Nix download/evaluation cache but places `NIXFIED_STATE_DIR` in ephemeral
runner storage. Rust compilation artifacts are therefore cold on each hosted
runner.

### Current Nix derivation boundary

Nix already avoids rebuilding the Nixfied runtime and generated application
launchers when their declared inputs are unchanged. That reuse does not
currently extend to MFM's verification artifacts.

The `check`, `test`, `test-db`, and `ci` applications are generated from the
Nixfied model and live in `/nix/store`, but they are launchers. At runtime they
execute Cargo against the live workspace and write mutable outputs to
`${stateDir}/cargo-target`. The `.rlib` files, proc macros, test binaries,
fingerprints, and incremental units are therefore unknown to the Nix derivation
graph.

The packaged `mfm` CLI in [flake.nix](flake.nix) is a real
`buildRustPackage` derivation and demonstrates exact derivation reuse. Its
current granularity is too coarse for verification caching:

- `src = ./.` makes the repository source tree one input, so an unrelated
  tracked-file change can change the derivation;
- the `cargoDeps` derivation contains vendored dependency sources, not compiled
  dependency artifacts;
- a changed MFM source derivation therefore recompiles the final Cargo graph;
  and
- it builds the release CLI, not the verification feature/profile/test matrix.

Nix can cache MFM compilation only when compilation itself becomes a declared
derivation output. Pointing Cargo at `/nix/store` is not a solution: the store
is immutable while Cargo target directories are mutable.

The local Nix store can safely share identical derivation outputs across
worktrees without an external cache. A fresh hosted CI runner has no local MFM
outputs, so cross-run CI reuse would still require a trusted binary cache or a
persistent runner. Platform-specific outputs also remain separate.

## Goals

This RFC has the following goals:

1. provide fast, focused feedback during normal development;
2. reduce the median duration and variance of comprehensive verification;
3. bound verification artifact size and inode growth;
4. remove duplicated work without reducing coverage;
5. preserve useful source-line diagnostics and backtraces;
6. preserve reproducibility with the Nix-pinned Rust toolchain;
7. make cache ownership, isolation, invalidation, and cleanup explicit;
8. measure compilation, execution, and service costs independently;
9. determine whether Nix-native compiled artifacts provide safe reuse across
   worktrees and verification runs; and
10. select the simplest long-term cache architecture that meets the measured
    performance, storage, correctness, and operational requirements.

## Non-goals

This RFC does not authorize:

- removing or weakening trybuild, doctest, metadata, SQLx, database, CLI, REST,
  keystore, or parity checks;
- merging crates solely to reduce Cargo package count;
- sharing one raw Cargo target directory across worktrees or build lanes;
- caching secret material or service state;
- changing CLI, REST, persisted-data, replay, or architecture contracts;
- treating static unused-dependency reports as automatic removal decisions;
- assuming that a Nix-stored launcher implies Nix-stored Cargo artifacts;
- introducing one derivation per workspace crate before simpler granularity is
  measured;
- caching successful test results merely because compiled artifacts are
  cacheable;
- accepting a faster build with worse correctness or materially worse
  diagnostics.

## Design constraints

Every implementation stage remains subject to
[the code-quality policy](docs/code-quality.md),
[the architecture taxonomy](docs/architecture.md), and
[the design contract](docs/design.md). In particular:

- library and binary crate boundaries remain intact;
- state logic does not gain ambient I/O to simplify testing or caching;
- append-only, atomic, content-addressed, and canonical-hashing contracts are
  unchanged;
- secrets must not enter manifests, events, artifacts, outputs, diagnostics,
  test fixtures, or caches;
- CLI and REST text and JSON contracts remain stable;
- security-sensitive keystore and signing behavior remains fully covered; and
- a missing cache-lifecycle or task-model capability must be added properly,
  not approximated with a fragile shell workaround.

## Decision

### 1. Establish two build lanes

#### Developer lane

The developer lane is optimized for edit-feedback latency:

- use the repository-pinned Rust environment through a development shell or an
  equivalent repository-owned entry point;
- retain incremental compilation and full development debugging information;
- use a target directory owned by the current worktree;
- prefer focused commands such as `cargo check -p <package>` and
  `cargo test -p <package>`;
- provide a non-gating `quick` entry point for formatting plus a broad type
  check when package-level selection is not convenient.

The initial `quick` contract should be equivalent to:

```text
cargo fmt --all -- --check
cargo check --workspace --lib --bins
```

The quick lane is not evidence that the comprehensive gates passed.

#### Verification lane

The verification lane is optimized for assurance, reproducibility, and bounded
artifacts:

- preserve `nix run .#check`, `nix run .#test`, `nix run .#test-db`, and
  `nix run .#ci` as the comprehensive entry points;
- use a Nixfied-owned, slot-scoped Cargo target distinct from direct Cargo;
- disable incremental compilation;
- retain line-table debug information but disable split-debug artifacts;
- cover every Cargo leaf, including nested Cargo launched by trybuild and SQLx;
- clean only the active build cache through an explicit scoped maintenance
  operation, never through ad hoc recursive deletion of Nixfied state.

The two lanes intentionally do not share raw Cargo targets. If cross-lane or
cross-worktree reuse is later introduced, it will share content-addressed
compiler outputs rather than mutable Cargo state.

### 2. Consolidate the workspace test execution

Workspace Nextest will explicitly select the required app feature:

```text
cargo nextest run --workspace --features mfm-app/test-support
```

The separate `mfm-app` test-support Nextest task and its composite dependency
will then be removed. Acceptance is based on comparing test identifiers and
required feature-specific tests at the same commit, not merely on comparing a
total count.

All nine trybuild harnesses, their UI cases, and the complete workspace
Nextest coverage remain mandatory.

### 3. Remove the redundant SQLx package clean

The explicit `cargo clean -p mfm-stream-store-postgres` will be removed from the
online SQLx task. SQLx will initially continue to use the shared verification
target so that its built-in minimal recompilation behavior can be measured
without paying for a second target tree.

Online SQLx preparation must remain uncached by any compiler-object cache. The
database schema is ambient proc-macro input that is not represented in a normal
compiler cache key. A cache hit that bypasses macro execution could incorrectly
accept stale SQLx metadata.

Acceptance must include a disposable-schema mutation test proving that
`cargo sqlx prepare --check` fails after a schema change when Rust sources have
not changed.

### 4. Enforce the verification artifact policy centrally

The initial verification policy is:

```text
CARGO_INCREMENTAL=0
CARGO_PROFILE_DEV_DEBUG=1
CARGO_PROFILE_TEST_DEBUG=1
CARGO_PROFILE_DEV_SPLIT_DEBUGINFO=off
CARGO_PROFILE_TEST_SPLIT_DEBUGINFO=off
```

`debug=1` retains line tables while avoiding full debug payloads. Applying the
policy in the Nixfied verification environment covers `cargo check`, Clippy,
builds, tests, doctests, SQLx-internal Cargo, and trybuild's nested development
builds. Direct developer Cargo remains unaffected.

This central policy is preferred over relying only on `--profile ci`, because
it is easy for an internal or nested Cargo invocation to omit the named
profile. The existing ineffective `[profile.ci]` should be removed or replaced
as part of this change so it cannot imply coverage it does not provide.

Before fixing additional profile values, benchmark:

- the nonincremental default `codegen-units` against `256`; and
- `opt-level=0` against `opt-level=1` for total compile-plus-test time.

An optimized test profile is accepted only when total gate time improves and
debug assertions, overflow checks, diagnostics, and test behavior remain
intact.

### 5. Use purpose-specific, bounded target caches

The Nixfied verification target should use Nixfied's first-class cache
environment mechanism rather than a single raw `${stateDir}/cargo-target`.
Its identity must include:

- operating system and target architecture;
- the exact Rust compiler/toolchain closure or version;
- the Nixfied slot or other concurrent-worktree isolation boundary; and
- an explicit verification-policy version.

`Cargo.lock` should not create a new local target-directory identity on every
dependency update. Cargo already fingerprints dependency changes inside the
target, while lock-keyed directories would leave large orphan caches behind.

Cache lifecycle must be explicit and scoped to the build cache. Cleanup must
not traverse or delete Postgres data, run records, parity logs, or other
Nixfied service and diagnostic state. If the required cache-family garbage
collection is missing, it should be added properly rather than approximated
with `find -delete` or broad slot deletion.

### 6. Evaluate Nix-native compiled artifacts

After task deduplication, profile selection, and the improved mutable-target
baseline are stable, MFM will test whether compilation should cross the Nix
derivation boundary.

#### Pilot shape

The first pilot will use coarse, intentional layers rather than immediately
generating a derivation for every workspace crate:

1. a dependency-artifact derivation that can remain unchanged across ordinary
   MFM source edits; and
2. one or more verification-artifact derivations producing the final binaries
   required by a declared feature/profile surface.

The verification outputs may include a Nextest archive or equivalent immutable
test-binary bundle, the MFM CLI used by parity tests, and other final binaries
that Nixfied can execute directly. Nixfied remains responsible for live
service lifecycle, runtime environment, task evidence, and test execution.

The pilot must not copy a prewarmed Cargo target tree from `/nix/store` into
mutable state. It should expose final immutable artifacts as declared Nixfied
closures. If first-class model support is missing, that support must be added
properly rather than approximated with shell copying or path rewriting.

Doctests and trybuild require explicit treatment. A Nextest archive alone does
not replace doctest execution, and trybuild test binaries can launch nested
Cargo compilations. Those checks may remain in the runtime Cargo lane or become
separate pure derivations, but the pilot must preserve their complete coverage
and report their cost separately.

#### Source and derivation correctness

The pilot must declare every compile-relevant input, including as applicable:

- workspace and package manifests and `Cargo.lock`;
- Rust sources, build scripts, proc-macro sources, and generated-source inputs;
- SQLx offline metadata and migrations consumed at compile time;
- files read through `include_bytes!`, `include_str!`, or equivalent paths;
- target, feature, profile, compiler, linker, and platform configuration; and
- environment values that can affect compilation.

Source filtering should exclude files proven not to affect compilation, so a
documentation-only edit does not rebuild Rust artifacts. It must fail toward
rebuilding rather than risk a false cache hit when an input is uncertain.

Online SQLx preparation remains a live, uncached task. Database schema and
service state must not be smuggled into an otherwise pure derivation.

#### Granularity decision

The initial dependency/final-artifact split should recover most third-party
compilation without generating a large Nix graph. A finer workspace-crate
derivation graph will be considered only if measurements show that unchanged
MFM crates still dominate rebuild time.

A per-crate design would need to preserve Cargo resolver behavior, feature
unification, host/target separation, build-script and proc-macro dependencies,
and reverse-dependency invalidation. Its evaluation must include Nix evaluation
time, derivation count, closure size, and garbage-collection behavior, not only
compile time.

The pilot may use an established Nix Rust build pattern such as a
dependency-only derivation, but selection of a new flake input or build
framework is part of the evaluation. This RFC does not authorize a dependency
solely because it can produce a successful prototype.

### 7. Evaluate compiler-object caching after the Nix pilot

After the Nix-native pilot establishes its reuse granularity and operational
cost, MFM will run a bounded `sccache` comparison on Linux when compilation
remains material:

- use the Nix-pinned `sccache` package;
- keep incremental compilation disabled in the verification lane;
- start with a bounded local cache of approximately 2 GiB;
- persist only the compiler-object cache in CI, never the Cargo target tree;
- key CI cache archives by platform, architecture, exact Rust/Nix toolchain,
  `Cargo.lock`, and cache-policy version;
- allow writes only from trusted workflows and never place secrets in cached
  inputs or outputs;
- disable the compiler cache for online SQLx preparation;
- retain a periodic cache-bypass CI run.

Build scripts, proc macros, linking, rustdoc, and actual test execution are not
all eliminated by compiler-object caching. The pilot will be rejected if its
transfer, lookup, and whole-crate recompilation overhead does not produce a net
improvement. It will not be combined with Nix-native compiled artifacts unless
an isolated hybrid measurement demonstrates additional value and a clear cache
ownership model.

### 8. Treat dependency and scheduling work as later optimizations

After build-policy changes have a controlled baseline, separate changes may:

- narrow Tokio's workspace `full` feature set where feature analysis proves a
  smaller set is correct;
- reconcile Reqwest feature variants where transport security and trust
  semantics remain unchanged;
- remove verified unused direct dependencies;
- reduce active duplicate transitive versions where upstream constraints
  permit it;
- change doctest or parity scheduling when measurements show available
  parallelism without Cargo locks, service conflicts, or CPU contention.

These changes must not be mixed into the build-policy rollout. Raw lockfile
duplication and package count are not sufficient justification.

## Gated implementation and commit protocol

This RFC is not authorization to execute the whole sequence in one work
session or pull request. A request to begin implementation authorizes only the
next approved phase.

### Mandatory stop rule

Each successful phase produces at most one logical commit. After evaluating
that clean commit, the engineer must stop, deliver the phase report below, and
wait for explicit owner approval before preparing or implementing the next
phase.

The stop applies even when:

- every test and performance threshold passes;
- the next change appears mechanical;
- the remaining token, time, or CI budget is available; or
- multiple later phases could be implemented together conveniently.

Do not start background measurements, edit files, add dependencies, or prepare
the next phase while awaiting approval. Approval for one phase does not imply
approval for another.

One phase/one commit is a success-path maximum, not a requirement to commit
broken or unjustified work:

- if a correctness or coverage check fails before commit, do not commit;
- if required underlying Nixfied support is missing, do not add a shell
  workaround; stop and report the capability gap;
- if post-commit evaluation finds a regression, stop and recommend keep,
  revert, or redesign without taking that action automatically;
- if an experiment has no qualifying implementation, record a report-only
  conclusion rather than manufacturing an empty or knowingly inferior code
  commit.

### Per-phase workflow

For every phase, in order:

1. Confirm explicit approval for that phase and its stated scope.
2. Confirm the worktree has no unrelated changes. Preserve all user-owned
   changes and stop if they overlap the phase.
3. Record the parent commit, toolchain, host, cache state, task graph, relevant
   baseline measurements, and test inventory.
4. Implement only the phase outcome. Experimental alternatives use isolated
   targets or worktrees and are not accumulated in the main worktree.
5. Run focused checks while developing and compare the relevant inventories
   before and after the change.
6. Before committing, run the repository-required gates:

   ```text
   nix run .#check
   nix run .#test
   nix run .#test-db
   ```

7. Run `nix run .#ci` for any phase that changes build tasks, test selection,
   SQLx behavior, profiles, cache behavior, Nix derivations, GitHub workflows,
   or the selected default architecture.
8. Create one commit with a lower-case subject and only the phase's logical
   change.
9. Evaluate the clean committed tree using the phase-specific matrix. Record
   exact commands, Nixfied run ids, timings, storage, coverage, and any
   unverified surface. Never report a dirty-worktree Nix result as an
   exact-derivation cache result.
10. Deliver the phase report and stop. Do not proceed until the owner explicitly
    selects proceed, hold, revert, or redesign.

Documentation-only commits remain subject to the repository's current
pre-commit gate policy. If that policy changes independently, this workflow
follows the new authoritative policy.

### Required phase report

Every stop report must be self-contained and use this shape:

```text
Phase:
Commit: <hash and lower-case subject, or "not committed">
Outcome: passed | failed | mixed | blocked

Changed:
- files and behavior changed
- explicit non-changes

Verification:
- focused checks and results
- check/test/test-db/ci results and durations
- test, doctest, trybuild, schema, and parity inventory comparison

Measurements:
- parent versus candidate timings under named cache states
- compile/link versus execution time
- Cargo target, Nix closure/store, inode, and cache statistics

Acceptance:
- each applicable threshold: pass | fail | not yet applicable

Risks and limitations:
- failures, variance, missing platforms, operational cost, and uncertainty

Recommendation:
- proceed | hold | revert | redesign
- exact scope requested for the next phase
```

Never include database credentials, private paths containing secrets, private
keys, mnemonics, or secret-bearing environment values in a report. Report
sanitized command shapes and Nixfied run identifiers instead.

### Phase and commit plan

#### Phase 00: land the accepted RFC

Change:

- commit this RFC as the sole documentation change;
- establish the gated execution and measurement contract; and
- make no build, workflow, or runtime behavior change.

Evaluate:

- validate Markdown whitespace and every local link;
- confirm the RFC distinguishes observations from controlled baselines; and
- run the repository-required pre-commit gates.

Expected commit subject:

```text
docs: define gated slow-build evaluation
```

Stop report decision: approve or revise the execution contract before any
performance implementation begins.

#### Phase 01: record the controlled baseline

Change:

- add one repository-owned baseline report under `docs/` containing the
  measurement context, commands, run ids, test inventory, and results;
- measure the clean Phase 00 commit, not the later report commit; and
- make no build configuration change.

Evaluate:

- three isolated clean runs and five warm runs where feasible;
- focused leaf, broad quick-equivalent check, each public Nixfied gate, and full
  CI;
- Linux and macOS, explicitly marking any platform not yet available; and
- compilation, linking, execution, service, target-size, `.dwo`, inode, and
  variance measurements separately.

Expected commit subject:

```text
docs: record controlled build baseline
```

Stop report decision: accept the baseline as decision-quality or repeat it
before changing behavior.

#### Phase 02: remove duplicate app test execution

Change:

- make `mfm-app/test-support` explicit in workspace Nextest;
- remove the separate app Nextest task and composite edge; and
- change no other test selection or ordering.

Evaluate:

- compare exact Nextest test identifiers before and after;
- prove the app tests and manual-resolution coverage remain present;
- run all nine trybuild harnesses and all discovered doctests; and
- measure removed execution and any compile/fingerprint difference.

Expected commit subject:

```text
remove duplicate mfm app test run
```

Stop report decision: keep, revert, or redesign the consolidation.

#### Phase 03: remove redundant SQLx invalidation

Change:

- remove MFM's explicit package clean from online SQLx preparation;
- add or strengthen a disposable-schema mutation check; and
- leave online SQLx uncached.

Evaluate:

- prove unchanged schema and checked metadata pass;
- prove a schema mutation with unchanged Rust sources fails;
- restore the disposable schema and prove the gate passes again; and
- measure which Cargo units rebuild before and after the change.

Expected commit subject:

```text
stop precleaning postgres sqlx artifacts
```

Stop report decision: keep only if drift detection and cleanup semantics remain
fail-closed.

#### Phase 04: add the developer lane

Change:

- expose the repository-pinned development environment;
- add the non-gating `quick` entry point; and
- keep all comprehensive gate commands and semantics unchanged.

Evaluate:

- focused package check/test latency;
- the quick command's exact target surface;
- toolchain equality with Nixfied verification; and
- proof that `quick` is neither called by nor substituted for a full gate.

Expected commit subject:

```text
add pinned quick development lane
```

Stop report decision: accept the inner-loop contract before changing
verification artifacts.

#### Phase 05: compact verification artifacts

Change:

- centrally disable verification incremental compilation;
- select line-table debug information and disable split-debug artifacts;
- remove or replace the ineffective `[profile.ci]`; and
- leave direct developer Cargo profiles unchanged.

Evaluate:

- clean and warm gate time;
- backtrace file/line quality on Linux and macOS;
- target, incremental, `.dwo`, inode, and trybuild sizes after two full runs;
- complete test and feature inventory; and
- rollback behavior when the policy version changes.

Expected commit subject:

```text
use compact verification build artifacts
```

Stop report decision: keep, revert, or adjust the verification artifact policy.

#### Phase 06: isolate mutable verification targets

Change:

- replace the undifferentiated target path with a Nixfied cache environment
  identity scoped by platform, toolchain, slot/worktree boundary, and policy;
- add only safe, build-cache-scoped lifecycle behavior; and
- do not alter Postgres, run-record, or diagnostic state cleanup.

Evaluate:

- reuse in repeated runs from one worktree;
- isolation between two concurrent worktrees and slots;
- compiler/toolchain and policy-version invalidation;
- cleanup protection for service and diagnostic state; and
- disk growth across repeated and changed-input runs.

Expected commit subject:

```text
scope verification cargo artifacts by slot
```

Stop report decision: if Nixfied lacks correct cache-family lifecycle support,
report the upstream capability needed instead of continuing locally.

#### Phase 07: select measured profile tuning

Change:

- compare codegen-unit defaults with `256` and `opt-level=0` with
  `opt-level=1` using isolated variants;
- commit only a qualifying winner plus its documented rationale; or
- commit a report-only rejection when the baseline remains best.

Evaluate:

- total compile, link, test, doctest, trybuild, and parity time;
- debug assertions, overflow checks, diagnostics, and backtraces;
- clean, warm, and one-leaf-edit behavior; and
- Linux/macOS consistency.

Expected commit subject, selected according to the result:

```text
tune verification profile from measurements
```

or:

```text
docs: record verification profile experiment
```

Stop report decision: freeze the winning baseline used by both cache pilots.

#### Phase 08: add an opt-in Nix dependency-artifact pilot

Change:

- add a non-default derivation that separates compiled third-party dependencies
  from final MFM artifacts;
- declare and regression-test compile-relevant inputs; and
- avoid changing public Nixfied gates.

Evaluate:

- exact source, documentation-only, leaf-crate, shared-crate, manifest/lock,
  feature/profile, and cross-worktree changes;
- derivations rebuilt versus reused;
- evaluation/build time and output/closure size; and
- source-filter false-hit and conservative-rebuild tests.

Expected commit subject:

```text
add nix dependency artifact pilot
```

Stop report decision: if a new Nix Rust framework or flake input is required,
report its justification and alternatives before adding it in a separately
approved phase.

#### Phase 09: add opt-in Nix verification artifacts

Change:

- build a declared feature/profile bundle containing the feasible final test
  binaries, Nextest archive or equivalent, and CLI artifact;
- keep the bundle opt-in; and
- do not copy a Cargo target from the Nix store.

Evaluate:

- binary and test inventory against the mutable-target baseline;
- runtime closure completeness and backtrace quality;
- exact-hit and source-change rebuild behavior;
- doctest and trybuild surfaces not represented by the bundle; and
- artifact and closure size.

Expected commit subject:

```text
add nix verification artifact bundle
```

Stop report decision: approve direct Nixfied consumption only after artifact
coverage and purity are understood.

#### Phase 10: execute Nix artifacts through shadow verification

Change:

- add an opt-in, non-public-gating Nixfied task that executes immutable test
  artifacts directly from declared closures;
- retain runtime Cargo tasks as the authoritative comparison; and
- leave live-service parity and online SQLx unchanged.

Evaluate:

- result and test-identifier equality with authoritative workspace tests;
- execution without writable target copying or undeclared host tools;
- task evidence and failure diagnostics; and
- exact-source and changed-source end-to-end wall time.

Expected commit subject:

```text
run nix artifacts through shadow verification
```

Stop report decision: approve or reject expansion into live-service parity.

#### Phase 11: execute Nix artifacts through shadow parity

Change:

- extend only the opt-in shadow path to parity tests whose binaries can be
  built purely;
- keep Postgres lifecycle and test execution under Nixfied; and
- keep online SQLx preparation live and uncached.

Evaluate:

- parity result, output, schema, and service-lifecycle equality;
- ambient-input isolation and SQLx drift behavior;
- compile versus service/test execution time; and
- closure portability on Linux and macOS.

Expected commit subject:

```text
run nix artifacts through shadow parity
```

Stop report decision: determine whether the Nix pilot has enough coverage for
a formal candidate assessment.

#### Phase 12: record the Nix-native pilot conclusion

Change:

- add a decision-quality Nix pilot report under `docs/`;
- classify each acceptance criterion and unresolved surface; and
- keep all pilot tasks opt-in.

Evaluate:

- persistent local store versus fresh runner behavior;
- binary-cache transfer and trust costs if tested;
- full source-change matrix and Linux/macOS results;
- storage, garbage-collection, evaluation, and maintenance cost; and
- whether finer per-crate derivations are justified by measured residual work.

Expected commit subject:

```text
docs: record nix artifact pilot results
```

Stop report decision: proceed to `sccache`, request a narrower Nix refinement,
or hold because compilation is no longer material.

#### Phase 13: add an opt-in local `sccache` pilot

Change:

- add the Nix-pinned compiler cache to an opt-in verification path;
- enforce its size bound and nonincremental profile; and
- explicitly disable it for online SQLx preparation.

Evaluate:

- cold, warm, one-leaf-edit, shared-crate, and cache-bypass results;
- cacheable requests, hits, misses, evictions, and lookup overhead;
- linking, rustdoc, trybuild, and execution time left uncached;
- target plus cache disk consumption; and
- result equality with both the target-only and Nix-native candidates.

Expected commit subject:

```text
add bounded local rust compiler cache pilot
```

Stop report decision: approve CI persistence only if local results meet the
pilot thresholds.

#### Phase 14: pilot `sccache` persistence in CI

This phase is conditional and requires separate approval after Phase 13.

Change:

- persist only the bounded compiler-object cache on Linux CI;
- key it by the declared platform, toolchain, lock, and policy inputs;
- restrict writes to trusted workflows; and
- retain a cache-bypass comparison.

Evaluate:

- fresh, restored, and bypass CI runs;
- upload/download time, hit rate, archive size, retention, and failure behavior;
- untrusted pull-request read/write boundaries and secret exclusion; and
- whether macOS deserves a separate later pilot.

Expected commit subject:

```text
pilot bounded rust compiler cache in ci
```

Stop report decision: classify `sccache` as eligible, rejected, or requiring a
bounded follow-up.

#### Phase 15: select the long-term architecture

Change:

- update this RFC or add an ADR with the comparable target-only, Nix-native,
  `sccache`, and hybrid results;
- select the least complex qualifying design, which may differ between
  persistent workstations and ephemeral CI; and
- define the rollout and rollback plan without enabling it yet.

Evaluate:

- every common correctness, performance, storage, security, diagnostic, and
  lifecycle criterion;
- operational dependencies and maintenance burden;
- sensitivity to cacheless and changed-input scenarios; and
- whether a hybrid's incremental gain justifies two cache models.

Expected commit subject:

```text
docs: select long-term build cache architecture
```

Stop report decision: explicit owner approval is required before changing the
authoritative verification path.

#### Phase 16: enable the selected architecture

Change:

- make only the approved candidate authoritative for the agreed environment;
- retain a documented bypass/rollback path; and
- preserve public gate names, coverage, and output contracts.

Evaluate:

- the full clean/warm/change matrix on Linux and macOS;
- cacheless and rollback runs;
- all comprehensive Nixfied gates and CI; and
- repeated-run storage and lifecycle behavior.

Expected commit subject:

```text
enable selected verification cache architecture
```

Stop report decision: accept the rollout, roll it back, or hold before cleanup.

#### Phase 17: remove rejected pilot surfaces and finalize the record

Change:

- remove opt-in tasks, packages, workflow branches, and dependencies belonging
  only to rejected candidates;
- retain reusable measurement support only when justified; and
- mark the RFC implemented with final measured results and operating guidance.

Evaluate:

- absence of dead tasks, flake inputs, cache directories, and workflow paths;
- model/schema, Cargo metadata, full gate, and CI correctness;
- documentation and command-contract consistency; and
- final disk, timing, and test-inventory comparison against Phase 01.

Expected commit subject:

```text
remove rejected build cache pilots
```

Stop report decision: final merge-readiness review. Dependency-feature,
unused-dependency, transitive-version, and scheduling optimizations are new
work streams and require their own approved phase/commit plans.

## Measurement protocol

Every performance decision must record:

- commit and worktree identity;
- operating system, architecture, CPU, memory, and storage context;
- exact Rust, Cargo, Nextest, SQLx CLI, Nix, and Nixfied versions;
- task graph and selected feature/profile surfaces;
- whether the target and compiler cache are clean, warm, or deliberately
  invalidated;
- compilation/link time separately from test and service execution;
- per-task wall time;
- target-directory size, incremental size, trybuild size, `.dwo` count, and
  file count;
- Nix evaluation, realization, build, substitution, and artifact-copy time;
- the set and count of Nix derivations rebuilt, reused locally, or substituted;
- Nix output and transitive closure sizes for MFM-specific artifacts;
- compiler-cache request, hit, miss, eviction, and transfer statistics when
  applicable.

Use fresh isolated targets for clean measurements instead of cleaning an
active developer or verification cache. At minimum, compare three clean runs
and five warm runs on Linux and macOS. Use medians for acceptance and retain
the distribution so cache variance is visible.

The baseline must also include:

- a focused leaf-package check;
- the broad developer quick check;
- each individual Nixfied gate; and
- the full CI composite.

Each cache candidate must be exercised against the same source-change matrix:

- exact unchanged source;
- a documentation-only tracked-file change;
- a change to one leaf workspace crate;
- a change to a widely shared workspace crate;
- a manifest or `Cargo.lock` change;
- a feature/profile change;
- a second worktree with identical source inputs; and
- a fresh environment with no MFM-specific local cache.

Nix-native measurements must distinguish a warm local store from a fresh
hosted runner. Where a binary cache is evaluated, upload, download,
substitution, signing, retention, and operational costs count toward the
result. `sccache` measurements must likewise include archive or remote transfer
cost rather than reporting compiler hit rate alone.

## Acceptance criteria

The staged solution is accepted only if it satisfies all correctness and
coverage requirements at the same commit.

### Correctness and coverage

- Workspace test identifiers are unchanged after deduplication, except for the
  removed duplicate execution.
- Required `mfm-app/test-support` tests, including manual-resolution coverage,
  remain present.
- All trybuild harnesses and UI cases continue to run.
- All discovered doctests, Cargo metadata contracts, SQLx checks, database
  parity tests, state-event checks, CLI tests, REST tests, and keystore parity
  tests continue to run.
- The disposable-schema mutation test proves that online SQLx drift is not
  hidden by caching.
- CLI and REST output contracts and persisted formats are unchanged.
- Verification failures retain actionable source file and line information.

### Performance and storage

- Focused warm leaf feedback has a median below 10 seconds on the reference
  Linux development host.
- The existing 1.77-second warm workspace check does not regress by more than
  10% under comparable conditions.
- Full-gate median wall time improves by at least 20% without increased
  flakiness.
- Verification artifact storage decreases by at least 40%, with a target of
  no more than 15 GiB after two complete runs.
- `.dwo` inode count decreases by at least 80%.
- A repeated unchanged full gate grows the target by less than 2%.

### Nix-native artifact pilot

The Nix-native design remains eligible only if:

- an exact unchanged derivation performs no Rust recompilation;
- ordinary workspace source edits reuse the compiled third-party dependency
  layer;
- changes to files proven not to be compile inputs do not invalidate Rust
  artifacts;
- all compile-relevant inputs are represented in derivation identity, with
  deliberate regression tests for build-script, included-file, SQLx metadata,
  target, feature, and profile changes;
- Nixfied executes final artifacts directly from declared immutable closures
  without copying or mutating a Cargo target tree;
- the same test identifiers and feature surfaces are exercised as in the
  mutable-target baseline;
- live-service and online SQLx behavior remains outside cached compilation;
- local-store and optional binary-cache lifecycles have explicit ownership,
  size bounds, trust policy, and garbage-collection behavior; and
- total verification time improves by at least 20% in a workload where
  compilation is material, after Nix evaluation, realization, and transfer
  overhead.

The initial pilot caches compiled artifacts, not successful test results.
Caching a pure test result would change verification execution semantics and
requires a separate decision.

### Compiler-cache pilot

`sccache` is adopted only if:

- median cold CI time improves by at least 20% after cache transfer overhead;
- the second-run hit rate for cacheable compiler requests is at least 50%, with
  60% or higher preferred;
- warm single-edit developer performance does not regress, because the
  compiler cache is not imposed on the incremental developer lane;
- a cache-bypass run produces the same results; and
- the configured size bound and trust policy are enforced.

### Long-term architecture decision

The final decision will use the complete results rather than assuming one
cache mechanism must serve every environment. It may select different
strategies for persistent developer workstations and ephemeral CI when the
tradeoff is explicit.

The chosen design must be the least complex candidate that satisfies coverage,
performance, storage, correctness, diagnostics, security, and lifecycle
requirements. A hybrid is selected only when its incremental benefit is
material after accounting for two invalidation models, two storage lifecycles,
and additional failure modes. The target-only baseline remains a valid winner
if the near-term fixes already meet the thresholds.

Failure to meet an acceptance threshold causes that stage to be rolled back or
redesigned without rolling back earlier independently successful stages.

## Risks and mitigations

### Reduced debug information

Line-table debug information is expected to preserve actionable backtraces but
does not provide the same debugger experience as full development debug data.
The developer lane retains full debugging information, and verification-profile
adoption requires explicit backtrace inspection on Linux and macOS.

### Nonincremental verification builds

Disabling incremental compilation can make some isolated rebuilds slower. It
is expected to reduce persistent size, invalidation complexity, and cold-CI
cache payloads. The decision is based on full-gate median time and storage, not
compile time alone.

### Nix source-input correctness

An incomplete source filter can produce a false Nix cache hit when a build
script, macro, SQLx query, or included file changes. The pilot must enumerate
and regression-test non-obvious inputs. When relevance cannot be proven, the
file remains an input and causes a conservative rebuild.

### Nix derivation granularity

A derivation that contains the whole repository rebuilds too much, while a
derivation per crate/feature/profile/target combination can create excessive
evaluation cost, closure growth, and maintenance complexity. Granularity is a
measured design variable. The rollout begins with dependency and final-artifact
layers and introduces finer units only when their benefit is demonstrated.

### Local-store and CI asymmetry

The local Nix store provides safe cross-worktree reuse on a persistent machine,
but fresh hosted CI runners do not retain MFM outputs. CI results must not claim
Nix reuse unless the required output was actually present or substituted from
a trusted binary cache, and the cost of that cache is included.

### Compiled artifacts versus test results

Reusing a test binary does not mean reusing the test's successful outcome.
Initial experiments continue to execute tests and parity workflows. Any future
proposal to cache pure test results must define trust, determinism, invalidation,
and evidence semantics separately.

### Compiler-cache correctness

Compiler caches do not automatically model ambient inputs. Online SQLx is
explicitly excluded, cache writes are restricted to trusted workflows, no
secrets may enter the cache, and cache-bypass verification remains available.

### Cache accumulation

Keyed caches can replace one unbounded directory with many orphan directories.
The design therefore requires bounded caches and scoped lifecycle management;
it does not create a new target for every `Cargo.lock` revision.

### Parallel task contention

Starting more tasks simultaneously may increase wall time on the reference
eight-core host or create service conflicts. Parallelism changes require
resource and isolation measurements rather than assuming that a wider DAG is
faster.

## Rejected alternatives

### Add `sccache` first

Rejected because it does not remove duplicate tests, SQLx invalidation, linking,
rustdoc work, or test execution. It may also make incremental single-edit
development worse. It remains a measured later-stage optimization.

### Assume Nix already caches Nixfied Cargo outputs

Rejected because Nix currently caches the launcher and tool closures, while
Cargo writes verification artifacts to mutable Nixfied state at runtime. Nix
cannot reuse outputs that are absent from its derivation graph.

### Treat the current `buildRustPackage` as sufficient granularity

Rejected because the packaged CLI uses the repository as one source input and
does not provide compiled dependency or verification-test layers. It proves
exact derivation reuse, but an ordinary source change still invalidates the
coarse final build.

### Generate one Nix derivation per workspace crate immediately

Rejected as the first experiment because Cargo features, host/target units,
build scripts, proc macros, profiles, tests, and reverse dependencies multiply
the graph. The dependency/final-artifact split must be measured before taking
on per-crate graph generation.

### Seed a mutable Cargo target from `/nix/store`

Rejected because Cargo expects to update its target, while Nix outputs are
immutable. Copying a prewarmed target would add transfer cost and recreate the
fingerprint, path, ownership, and cleanup problems this RFC is intended to
remove.

### Share one Cargo target everywhere

Rejected because target trees contain path-, profile-, feature-, and
toolchain-sensitive mutable state and locks. Cross-worktree sharing caused
accumulation and invalidation rather than dependable reuse.

### Put SQLx in a second target immediately

Rejected as the first step because SQLx still touches source mtimes in the
shared checkout and a second target forces duplicate compilation. Remove the
redundant clean and measure before considering stronger isolation.

### Globally cache online SQLx preparation

Rejected because database schema is ambient proc-macro input. A compiler cache
hit could bypass the macro execution required to detect schema drift.

### Remove compile-fail, doctest, or parity coverage

Rejected because these checks enforce public typestate, documentation,
metadata, persistence, and live-service contracts. The solution optimizes how
they run, not whether they run.

### Merge crates to reduce Cargo package count

Rejected because MFM's crate boundaries encode the architecture taxonomy and
keep libraries reusable without binary coupling. Package-count reduction is
not worth weakening those boundaries.

### Treat dependency updates as the primary remedy

Rejected because much of the resolved duplication is inactive on a given host,
and dependency/feature changes can alter security or platform semantics. They
belong in small, verified follow-ups.

### Broad or ad hoc cleanup

Rejected because Nixfied state also contains service data and diagnostic run
artifacts. Cleanup must target a known build-cache family and must not use
fragile recursive deletion rules.

## Verification governance

The developer lane is non-gating. Until
[repository policy](AGENTS.md) is deliberately changed, agents and contributors
must still run `nix run .#check`, `nix run .#test`, and `nix run .#test-db`
before each commit and `nix run .#ci` for major work or final merge-readiness
validation.

If MFM later wants fast multi-commit local workflows, gate frequency should be
changed explicitly in `AGENTS.md` and enforced at push, review, or merge time.
That governance decision is separate from this RFC's build architecture and
must not be inferred merely from the existence of `quick`.
