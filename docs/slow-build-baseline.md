# Controlled slow-build baseline

Status: Phase 01 baseline and Phase 02/03/04 follow-up for RFC_SLOW_BUILDS.md

This report measures the clean Phase 00 RFC commit
`51a5f9a07808cd9a92b218a028dd2424c04ac76d` (`docs: rfc builds tt3`). The
report itself is not included in the measured commit. No build configuration,
workflow, runtime, or test-selection changes were made during the measurement.

## Measurement context

- Host: Linux `aarch64`, 8 CPUs, 15 GiB RAM, 193 GiB filesystem; the host is
  a Lima VM. macOS was not available.
- Toolchain: Nix-pinned Rust and Cargo 1.96.0.
- Nix: 2.34.7.
- Nextest: 0.9.140.
- SQLx CLI: 0.9.0.
- Nixfied lock revision: `b0681e45ab76d5023d9c5e033087d34adf98e90b`.
- Nixfied model hash: `7eeede273c41e1e4ffde9d6b91bf472560039ecc70c5fd9229fcd791e341fd4f`.
- Nixfied runtime ABI: `nixfied-runtime-abi:1-5ff3aa14f2bf`.
- Workspace: 53 packages and 100 targets.
- Direct-Cargo clean measurements used fresh temporary target directories.
- Nixfied clean measurements used fresh slots 2, 3, 4, 5, and 7. Warm
  measurements reused their corresponding slot. Database runs were serialized
  and the slot was brought down after each run.
- The additional confirmation runs used a detached worktree at the exact Phase
  00 commit; measurements from the later report commit were not counted.
- No compiler-object cache was enabled.

The task graph was unchanged from the RFC commit:

- `check`: format, Clippy, Cargo metadata contract, and SQLx offline check.
- `test`: workspace Nextest, separate `mfm-app` test-support Nextest, and
  workspace doctests.
- `test-db`: SQLx preparation, CLI build, and four Postgres parity tasks.
- `ci`: `check`, `test`, the keystore parity task, and `test-db`.

## Test inventory

- Workspace Nextest: 974 tests across 99 binaries; 974 passed in successful
  runs.
- `mfm-app` test-support Nextest: 70 tests across 4 binaries; 70 passed.
- Workspace doctests: 53 doc-test binaries and 42 doctests passed in the clean
  CI run.
- Trybuild: 9 harnesses, 80 Rust UI cases, and 71 checked stderr baselines.
- SQLx, metadata, CLI, REST, state-event, collect/report, and keystore parity
  tasks were present in the baseline graph and passed in successful gate runs.

The separate app test-support task is intentionally recorded as duplicated
baseline work. Phase 02 may remove it only after comparing exact identifiers.

## Commands and results

The measurement command shapes were:

```text
CARGO_TARGET_DIR=<isolated> cargo check -p mfm-program
cargo fmt --all -- --check
CARGO_TARGET_DIR=<isolated> cargo check --workspace --lib --bins
nix run .#check -- --slot <slot>
nix run .#test -- --slot <slot>
nix run .#test-db -- --slot <slot>
nix run .#ci -- --slot <slot>
```

### Focused leaf check

Three isolated clean runs took 3.93s, 3.41s, and 3.69s. Five warm runs took
0.08s, 0.08s, 0.08s, 0.07s, and 0.08s. The clean median was 3.69s and the
warm median was 0.08s.

### Developer quick-equivalent check

Each run included format verification and
`cargo check --workspace --lib --bins`.

| State | Samples | Format wall time | Cargo check wall time | Combined wall time |
| --- | --- | --- | --- | --- |
| Clean | 3 | 1.04–1.11s | 18.60–20.30s | 19.67–21.34s |
| Warm | 5 | 0.99–1.19s | 0.17–0.33s | 1.16–1.38s |

### Individual Nixfied gates

| Gate | Clean runs (wall seconds) | Warm runs (wall seconds) |
| --- | --- | --- |
| `check` | `1856820-1784074810922092461` 67.67, `2852614-1784082527752127106` 69.10, `2869708-1784082603901086729` 57.93 | `1966169` 12.09, `1968491` 2.30, `1968860` 2.44, `1969178` 1.87, `1969491` 2.06 |
| `test` | `1871848-1784074882189534658` 317.85, `2885494-1784082672576615130` 222.39, `2935572-1784082903739716201` 217.23 | `1970364` 156.24, `1994349` 122.82, `2015584` 119.82, `2030190` 114.70, `2042694` 98.68 |
| `test-db` | `1938991-1784075212147378361` 206.44, `2985203-1784083130956263240` 143.70, `3002858-1784083286971702086` 140.97 | `2053592` 110.82, `2059015` 105.54, `2063472` 103.04, `2068401` 106.16, `2079260` 114.18 |

All listed individual gate runs passed. The warm `test` series overlapped a
separate user-owned MFM2 run for part of its measurement and is therefore
retained as observed data but should not be used as an uncontended performance
median. The three-clean/five-warm Linux matrix is complete for every listed
gate.

### Full CI

The first clean attempt, `2092179-1784076701902506403`, failed during Clippy
with `REGISTRY_CORRUPT: database or disk is full`. After scoped cleanup of the
measurement slots, clean run `2097869-1784076857019348816` passed all 14 tasks
in 452.26s.

The successful clean CI task breakdown was:

| Surface | Task wall-time total | Compile/link evidence | Execution/service evidence |
| --- | ---: | --- | --- |
| Check | 55.86s | Clippy and metadata compilation are included in task time | Format and contract execution are included |
| Workspace tests | 254.23s | Nextest test-profile compile 59.33s; app test-support compile 15.99s; doctest compile 6.13s | Nextest execution 133.01s; app execution 21.95s; doctest execution is included in its 22.35s task |
| Keystore parity | 10.29s | Included in task time | Included in task time |
| Database/parity | 129.64s | CLI build finished in 19.05s; parity test compiles were 6.87–14.86s | Postgres lifecycle, SQLx, and parity execution are included in task time |

Four warm CI runs passed:

- `2179721-1784077329975372989`: 240.23s
- `2202533-1784077570632039319`: 246.72s
- `2226810-1784077817921717921`: 244.48s
- `2248138-1784078062933119773`: 262.92s

The exact-parent completion runs also passed:

- `3020531-1784083443008166138`: clean, 382.39s
- `3113183-1784084140706080116`: clean, 372.10s
- `3083405-1784083828518983086`: warm, 246.09s

The fifth warm attempt, `2274840-1784078326405945267`, was intentionally
terminated after 164.14s to avoid another disk-full condition. It is not a
successful sample. There are now three successful clean and five successful
warm full-CI samples. Separate service startup time and linker time were not
instrumented independently; the task and Cargo profile timings above are the
available decomposition.

## Artifact and Nix measurements

After the clean CI run and four successful warm CI runs, slot 5 contained:

- Cargo target: 18 GiB.
- `debug/incremental`: 8.0 GiB.
- Nested trybuild target: 2.1 GiB.
- Files: 152,618.
- `.dwo` files: 98,241, occupying approximately 7.08 GiB.
- Nixfied slot state: 18 GiB.

The direct-Cargo temporary targets were approximately 131 MiB for the leaf
checks and 1.1 GiB for each broad quick-check target. They were removed after
measurement.

The realized `ci` launcher output was approximately 1.59 GiB; its transitive
Nix closure was 124 paths totaling approximately 12.22 GiB. The model-check
evaluation/realization took 1.44s. The outer Nixfied wall times include Nix
evaluation and realization; those costs were not separately instrumented for
each task. Warm runs reused the realized Nix launcher locally.

The repeated unchanged CI target grew from approximately 13 GiB after its
clean run to 18 GiB after four warm runs. This is an observation for the
pre-optimization baseline, not an acceptance judgment for a later phase.

The exact-parent confirmation slot measured 19 GiB after one clean and one
warm full-CI run, including 8.7 GiB of `debug/incremental`, 2.1 GiB of
trybuild artifacts, 143,580 files, and 82,548 `.dwo` files totaling
approximately 5.19 GiB. This uses a different isolated slot and is retained
as a second storage observation rather than merged into the slot-5 series.

## Limitations and conclusion

- Linux was the only available platform; no macOS measurements were possible.
- The three-clean/five-warm matrix is complete on Linux for the focused
  workload, quick-equivalent check, each individual Nixfied gate, and full CI.
- Full CI has three clean passes and five successful warm passes; the initial
  clean disk-full failure and intentionally canceled fifth warm attempt are
  recorded rather than hidden.
- The host filesystem reached 100% during the first clean CI attempt. Cleanup
  used only Nixfied's scoped `clean` operation on measurement slots 2–9 and
  temporary targets owned by this measurement. Slot 0, slot 1, and unrelated
  service state were not cleaned.
- Compilation versus test execution is available for Nextest and app
  test-support from their logs. Linking, doctest execution, and Postgres
  service startup were not separately timed.

This report establishes a complete Linux baseline for the later build-policy
phases. macOS remains unavailable and must be recorded as an unverified
platform until a macOS runner is available.

## Phase 02 follow-up: remove duplicate MFM app test run

The Phase 02 implementation used the expected subject `remove duplicate mfm app
test run`, with the Phase 01 report commit as its parent. The change made
`mfm-app/test-support` explicit in workspace Nextest and removed the separate
app-support task and composite edge.

The pre-change workspace list contained 974 identifiers with canonical hash
`bb5dbb8f080225104b31a0ae369db94ad4e8bc4c48213efb72c5dc48f817c2dd`. The
separate app-support list contained 70 identifiers with hash
`e398de989b467c3205ff70af300958ee7e15d50daaec0fe2130e8e0c47ec14b0`; its
union with the workspace list was still 974 identifiers and the workspace
hash. After the change, workspace Nextest with `mfm-app/test-support` again
contained 974 identifiers with the same hash, including
`mfm-app::manual_resolution`.

The changed gates passed on Linux `aarch64`:

- `nix run .#model-check`: model hash `6936ca7b9d3264c8940f29c3ec02a4e5d16ff1cec2da7427af8c181951c7b074`.
- `nix run .#check -- --slot 7`: 4/4, 50.80s, run `3287287-1784085380130104204`.
- `nix run .#test -- --slot 7`: 2/2 leaves, 178.48s, run `3302336-1784085434207388090`.
- `nix run .#test-db -- --slot 7`: 6/6, 123.07s, run `3340323-1784085616938741564`.
- `nix run .#ci -- --slot 7`: 13/13, 213.99s, run `3347233-1784085745199734840`.

The test composite therefore changed from three workspace-test leaves to two,
and CI changed from 14 tasks to 13. The post-change timings reused the
verification target and are not a controlled Phase 02 performance matrix;
they record removed execution and successful coverage. The Phase 01 artifact
and Nix storage measurements remain the authoritative storage baseline.

## Phase 03: stop precleaning postgres sqlx artifacts

The Phase 03 implementation uses the expected subject `stop precleaning
postgres sqlx artifacts`. Its parent is the Phase 02 commit
`6caeb099022ac018a409d872434e7c3deb1266b4`. The only behavior change is in
the online `postgres-sqlx-check` task in `nixfied.nix`: the explicit
`cargo clean -p mfm-stream-store-postgres` was removed, and the task now
performs three checks against a disposable schema. It first accepts the
unchanged migrated schema, drops `_sqlx_migrations.checksum` and requires
the unchanged Rust sources to reject preparation for that specific column
mutation, then recreates and migrates the schema and requires preparation to
pass again. The cleanup trap remains in place. No compiler-object cache or
other online SQLx cache was added.

The final-code verification was run on Linux `aarch64` with the pinned
toolchain and slot 7:

- `nix run .#model-check`: model hash
  `6fbe540c52e997047a206a056c0b676198729c165a45e2720b07192000a5becf`.
- `nix run .#check -- --slot 7`: 4/4, 6.16s, run
  `3499517-1784108882040720536`.
- `nix run .#test -- --slot 7`: 2/2, 158.81s, run
  `3501086-1784108892168141880`.
- `nix run .#test-db -- --slot 7`: 6/6, 101.12s, run
  `3494206-1784108776818048520`.
- `nix run .#ci -- --slot 7`: 13/13, 226.10s, run
  `3528257-1784109052177922449`.

The SQLx task log records `ALTER TABLE`, the expected mutation rejection,
schema drop/recreation, both migrations, and restored-schema acceptance. The
warm final-code SQLx leaf took 2.78s in the focused database run; its
successful baseline and restored checks each recompiled only
`mfm-stream-store-postgres` (0.60s and 0.43s). The expected failing mutation
check's compiler output is held in shell memory and is not emitted. The
earlier parent warm observation with the explicit clean, run
`2398478-1784079349884666509`, removed 2,793 files / 556.3 MiB and rebuilt
only `mfm-stream-store-postgres` in 3.63s; its SQLx task took 4.63s. These are
named warm observations, not a controlled full performance matrix. The
candidate therefore removes the redundant package-clean side effect while
retaining package-level invalidation for the changed database environment.

The test inventory is unchanged from Phase 02: workspace Nextest remains 974
tests across 99 binaries; doctests remain 53 binaries and 42 doctests;
trybuild remains 9 harnesses, 80 UI cases, and 71 checked stderr baselines;
the SQLx package retains 2 query macro sites and 2 `.sqlx` metadata files;
the database composite remains 6 leaves; and CI remains 13 tasks. No test,
doctest, trybuild, parity, or feature-selection surface changed.

At the end of the final candidate run, slot 7 contained an 18 GiB Cargo
target/state tree: `debug` was 16 GiB, `debug/incremental` 7.7 GiB,
trybuild artifacts 2.1 GiB, 120,917 files, and 69,015 `.dwo` files totaling
4.31 GiB. The realized CI launcher output was 12 KiB; its 124-path Nix
closure summed to 1,594,574,192 NAR bytes and occupied 1.6 GiB on disk. No
compiler-object cache was enabled. These storage observations are not a
Phase 03 acceptance threshold.

All Phase 03 acceptance checks pass: unchanged schema and checked metadata
pass; the checksum mutation with unchanged Rust sources fails and is
classified as the expected column failure; the recreated schema passes
again; cleanup is scoped to the disposable schema; and online SQLx remains
uncached. Linux is the only verified platform, and the warm timing comparison
has normal slot/cache variance. There is no numeric Phase 03 latency or
storage threshold in the RFC; the required unit-rebuild measurement and
fail-closed correctness evidence are complete.

The committed tree was then evaluated without worktree changes. The clean
`nix run .#model-check` reproduced the model hash above, and clean
`nix run .#test-db -- --slot 7` passed 6/6 in 97.81s under run
`3558651-1784109421683769708`. Its SQLx log again recorded the mutation
rejection and restored-schema acceptance; the warm successful prepares
recompiled only `mfm-stream-store-postgres` in 0.65s and 0.43s. This is the
phase-specific post-commit evaluation; the full required gates and CI were
run on the identical final source before the commit.

## Phase 04: add the developer lane

The Phase 04 candidate is based on the clean Phase 03 commit
`e94f09c644d6a001df1fcb58e3e3b9ba1c15ec46`. It adds a repository-pinned
`devShells.default` and the `.#quick` app/package in `flake.nix`, and documents
them in `README.md`. The shell and quick app reuse `nix/rust-toolchain.nix`,
Cargo Nextest, SQLx CLI, Git, pkg-config, and the native C toolchain. Their
shell hook/app unset `CARGO_TARGET_DIR`, so Cargo owns the normal `target`
directory of the current worktree; no verification target is shared. Cargo's
normal development profile remains incremental with full development debug
information.

The quick contract is exactly:

```text
cargo fmt --all -- --check
cargo check --workspace --lib --bins
```

It is a standalone flake app and is not a Nixfied task. The compiled Nixfied
model remains unchanged at hash
`6fbe540c52e997047a206a056c0b676198729c165a45e2720b07192000a5becf`; its
task model contains no `quick` task.

The parent direct-Cargo warm observations on the existing worktree target
were: `cargo check -p mfm-program` 2.94s, `cargo test -p mfm-program --lib`
5.85s, format 0.95s, and broad `cargo check --workspace --lib --bins`
10.24s. The Phase 04 developer shell used the same pinned Rust/Cargo 1.96.0
toolchain, and its focused package check/test took 2.63s/3.96s. The first
`nix run .#quick` took 9.22s including Nix app realization and the broad
Cargo check; a warm quick run took 6.66s, with Cargo's broad check finishing
in 5.36s. These are named warm/realization observations, not a controlled
performance threshold.

The final-code comprehensive gates passed on Linux `aarch64`, slot 7:

- `nix run .#model-check`: unchanged model hash above.
- `nix run .#check -- --slot 7`: 4/4, 61.79s, run
  `3752493-1784116230565407824`.
- `nix run .#test -- --slot 7`: 2/2, 201.93s, run
  `3767570-1784116297983467642`.
- `nix run .#test-db -- --slot 7`: 6/6, 131.09s, run
  `3806038-1784116508979241289`.
- `nix run .#ci -- --slot 7`: 13/13, 217.28s, run
  `3813018-1784116651292329201`.

The test inventory is unchanged: 974 workspace Nextest tests across 99
binaries, 53 doctest binaries with 42 doctests, 9 trybuild harnesses with 80
UI cases and 71 checked stderr baselines, 6 database leaves, and 13 CI tasks.
No feature, ordering, parity, SQLx, or service behavior changed.

The developer target grew from the parent observation of 11 GiB / 49,537
files to 12 GiB / 57,646 files after the focused and quick runs. The candidate
target contained 7.2 GiB of incremental artifacts, 1.2 GiB of trybuild
artifacts, 22,280 `.dwo` files totaling 1.25 GiB, and `debug` totaled 11 GiB.
The quick launcher output was 12 KiB with a 110-path closure totaling
1,506,420,728 NAR bytes and 1.5 GiB on disk. The development-shell output
was 80 KiB with a 137-path closure totaling 1,531,319,936 NAR bytes and
1.5 GiB on disk. Nixfied slot 7 measured 18 GiB after the comprehensive
gates. No compiler-object cache was present; the Cargo registry and Git
caches were 1.1 GiB and 1.3 MiB.

All Phase 04 acceptance checks pass: the pinned developer environment exposes
the same Rust/Cargo versions as Nixfied; the quick app executes exactly its
two non-gating commands; its target is worktree-owned and retains incremental
development artifacts; the quick app is absent from the Nixfied model; and
all comprehensive gate commands retain their prior graph and semantics.
Linux is the only verified platform, and no numeric Phase 04 latency
threshold is specified.

The committed tree was then evaluated without worktree changes. Clean model
admission reproduced the same hash, clean `nix develop` exposed Rust/Cargo
1.96.0 with empty `CARGO_TARGET_DIR` and `CARGO_INCREMENTAL`, and clean
`nix run .#quick` passed in 2.30s after the target was warm. This confirms the
developer lane and quick contract on the committed tree; slot 7 was then
stopped and cleaned through Nixfied's scoped lifecycle operations.
