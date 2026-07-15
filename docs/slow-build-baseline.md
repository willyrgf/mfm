# Controlled slow-build baseline

Status: Phase 01 baseline and Phase 02/03/04/05 follow-up for RFC_SLOW_BUILDS.md

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

## Phase 05: compact verification artifacts

The Phase 05 candidate is based on the clean Phase 04 commit
`76a0a82176df24410305e71f82603168a2e0ee2d` (`add pinned quick development
lane`). The implementation uses the expected subject `use compact verification
build artifacts`. The initial implementation commit before this report-only
metadata append was `8ae1ae25f038ae5f84c6e55f2571858239f62a05`; the final
amended commit is the same logical Phase 05 change.

The change is limited to two files:

- `Cargo.toml` no longer declares the ineffective `[profile.ci]` profile.
- `nixfied.nix` centrally applies the RFC's verification policy to every
  Nixfied Cargo leaf and its nested Cargo invocations:

  ```text
  CARGO_INCREMENTAL=0
  CARGO_PROFILE_DEV_DEBUG=1
  CARGO_PROFILE_TEST_DEBUG=1
  CARGO_PROFILE_DEV_SPLIT_DEBUGINFO=off
  CARGO_PROFILE_TEST_SPLIT_DEBUGINFO=off
  ```

  `debug=1` retains line tables; split-debug output and incremental compiler
  artifacts are disabled. The direct `[profile.dev]` and `[profile.test]`
  settings remain unchanged, as do `CARGO_TARGET_DIR`, the task graph, service
  lifecycle, SQLx behavior, test selection, and the developer lane.

### Parent and candidate verification

The Linux `aarch64` parent context was the pinned Rust/Cargo 1.96.0 toolchain,
Nixfied runtime ABI `nixfied-runtime-abi:1-5ff3aa14f2bf`, and model hash
`6fbe540c52e997047a206a056c0b676198729c165a45e2720b07192000a5becf`. No
compiler-object cache was present. The parent full-CI clean run
`run-3851733-1784117244059101633` passed 13/13 in 398.11s. Its post-run target
was 17 GiB, with 15 GiB in `debug`, 6.8 GiB in `debug/incremental`, 2.1 GiB
in nested trybuild artifacts, 98,462 files, and 52,635 `.dwo` files totaling
2.98 GiB.

The first parent warm run, `run-3914368-1784117659100948595`, took 57.52s but
failed one timing-sensitive test:
`mfm_core::keystore::tests::filesystem_tests::test_auto_lock_timeout_refreshes_on_sensitive_operations`
at `crates/core/src/keystore/filesystem_tests.rs:322`; 927/928 tests passed
and 46 were canceled. A single retry passed all 13 tasks in 186.01s under
`run-3923761-1784117748531381724`. The resulting parent target was 18 GiB,
with 16 GiB in `debug`, 7.7 GiB in `debug/incremental`, 2.1 GiB in trybuild,
116,674 files, and 64,724 `.dwo` files totaling 3.94 GiB. The failed attempt
is retained as observed scheduling variance and is not used as a successful
performance sample.

The candidate model admission passed with hash
`671a8642bd1436e705771a567f5aafed46b98de26dc919bf0ac546f87fc228d1`.
The required gates passed on the dirty candidate before commit:

- `nix run .#check -- --slot 7`: 4/4 in 48.39s,
  `run-3937479-1784117991138124567`.
- `nix run .#test -- --slot 7`: 2/2 in 175.24s,
  `run-3947577-1784118043061188390`.
- `nix run .#test-db -- --slot 7`: 6/6 in 104.37s,
  `run-3972481-1784118222441761281`.

After Nixfied slot 7 was cleaned, the two required full candidate runs passed
13/13:

- clean: `run-3974791-1784118343065006887`, 307.13s;
- warm: `run-4011750-1784118670744907098`, 197.49s.

The clean candidate Nextest task compiled the test profile in 37.51s and ran
974 tests in 100.47s. The warm task compiled in 16.73s and ran the same 974
tests in 73.05s. The remaining task timings, including format, Clippy,
metadata, SQLx, doctests, CLI, and Postgres parity, are retained in the two
Nixfied run summaries. Full-CI wall time was 90.98s lower on the clean
candidate than the parent clean sample; the successful warm candidate was
11.48s slower than the successful parent retry, so warm timing remains subject
to service and scheduling variance.

### Test and feature inventory

The candidate inventory is unchanged from Phase 04 and complete for the
selected graph:

- Workspace Nextest: 974 tests across 99 binaries; 974 passed.
- Workspace doctests: 53 doctest binaries and 42 doctests passed.
- Trybuild: 9 harnesses, 80 Rust UI cases, and 71 checked stderr baselines.
- Database/parity coverage: 6 database leaves; CI contains 13 tasks.
- Declared package features: `mfm/parity-tests`,
  `mfm-app/test-support`, `mfm-integration-tests/parity-tests`,
  `mfm-rest-api/parity-tests`, `mfm-stream-store-postgres/parity-tests`,
  `mfm-store/test-support`, and `mfm_core/default` plus
  `mfm_core/dangerous-secret-export`.
- Active gate feature selection remains `--all-features` for Clippy,
  `mfm-app/test-support` for workspace Nextest, and `parity-tests` for SQLx
  and parity tasks. No package, feature, test, doctest, trybuild, ordering, or
  service-selection surface changed.

### Artifact and cache measurements

The exact two-full-run artifact matrix was measured before the focused
diagnostic probe added one test binary to the warm target:

| State | Full CI | Cargo target | `debug` | `debug/incremental` | trybuild | files | `.dwo` files | `.dwo` size |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Parent clean | 398.11s | 17 GiB | 15 GiB | 6.8 GiB | 2.1 GiB | 98,462 | 52,635 | 2.98 GiB |
| Parent warm retry | 186.01s | 18 GiB | 16 GiB | 7.7 GiB | 2.1 GiB | 116,674 | 64,724 | 3.94 GiB |
| Candidate clean | 307.13s | 9.8 GiB | 8.1 GiB | 4 KiB directory, no artifacts | 1.7 GiB | 17,791 | 0 | 0 |
| Candidate warm | 197.49s | 9.8 GiB | 8.1 GiB | 4 KiB directory, no artifacts | 1.7 GiB | 17,791 | 0 | 0 |

The candidate warm target had 20,932 filesystem entries when measured with
`find -printf`; the parent file counts above are the preserved baseline metric
and did not include a separate all-entry inode capture. After the focused
backtrace probe, the candidate target was 10 GiB with 18,227 files, still with
no `.dwo` files and no incremental artifacts. The direct developer target
remained 12 GiB with 11 GiB in `debug`, 7.2 GiB in `debug/incremental`, 1.2
GiB in trybuild artifacts, 57,646 files, and 22,280 `.dwo` files totaling
1.25 GiB. This confirms the verification environment did not alter direct
developer Cargo output.

The realized candidate CI launcher was 12 KiB. Its Nix closure remained 124
paths, with 1,594,576,408 NAR bytes and approximately 1.6 GiB on disk. Cargo
registry and Git caches were 1.1 GiB and 1.3 MiB. No `sccache`, `ccache`, or
other compiler-object cache was enabled.

### Diagnostics and rollback

The focused candidate run of the previously timing-sensitive auto-lock test
passed 1/1 in 0.83s with `RUST_BACKTRACE=1`. It did not generate a panic, so
there is no new candidate failure trace to compare. The candidate test binary
contains `.debug_line` and decoded source line entries, and the parent
transient failure had already demonstrated file/line reporting at
`crates/core/src/keystore/filesystem_tests.rs:322`. This is a structural and
behavior-preserving Linux backtrace check; macOS was unavailable and remains
unverified.

Phase 05 does not introduce an explicit policy-version key; that identity is
the Phase 06 cache-scope change. As a disposable rollback proxy, a focused
`mfm-program` check in a temporary target took 3.07s on the first
`debug=1` policy, 0.05s warm, and 1.89s after changing the policy to
`debug=2`. The changed-policy run recompiled affected units and produced no
`.dwo` files, proving Cargo fingerprints do not silently reuse incompatible
profile artifacts. No temporary target was retained. No additional codegen or
optimization profile values were changed, so the RFC's future benchmark of
those alternatives is not applicable to this phase.

All applicable Phase 05 acceptance checks pass: the centralized policy covers
verification and nested Cargo; line tables remain while split and incremental
artifacts disappear; direct development profiles remain unchanged; the full
test, feature, doctest, trybuild, SQLx, and parity inventory is preserved; and
the policy toggle invalidates a disposable target. The Linux-only limitation,
the one parent timing-flake retry, and the lack of an explicit versioned cache
identity are recorded risks. No Nixfied workaround or missing support was
required.

The committed tree is evaluated below after the single logical commit.

The clean committed-tree evaluation of the initial Phase 05 commit passed
without a dirty-tree warning:

- `nix run .#model-check`: model hash
  `671a8642bd1436e705771a567f5aafed46b98de26dc919bf0ac546f87fc228d1`.
- `nix run .#ci -- --slot 7`: 13/13 in 197.96s,
  `run-4058941-1784119789680531480`.

The report-only metadata was then amended into the same logical commit; no
source, profile, task, feature, or workflow behavior changed in that amend.

## Phase 06: isolate mutable verification targets

Phase 06 was explicitly re-scoped by the owner to permit manual cleanup of
only the exact runtime-reported Cargo cache digest during measurement. This
does not add a project cleaner or replace the missing Nixfied cache-family
lifecycle operation. Broad slot cleanup remains outside the experiment.

The candidate change uses Nixfied `invocation.cacheEnv.CARGO_TARGET_DIR` for
every Cargo leaf. The cache is the `cargo-target` family with `slot` scope and
key parts for `cargo-target-v2`, `aarch64-unknown-linux-gnu`, Rust 1.96.0, and
`verification-v2`. The runtime therefore owns the cache placement below the
caller-selected state root and slot. Separate worktrees must select separate
`NIXFIED_STATE_DIR` values. The Cargo profile policy, task graph, services,
SQLx behavior, test selection, and direct developer target are unchanged.

### Phase 06 cache identity

The clean candidate model hash is
`d25c647af500ec8c060e173f05e199564cf3d3921663c82f04c3477a4c49ce84`, with
runtime ABI `nixfied-runtime-abi:1-5ff3aa14f2bf` and Rust toolchain 1.96.0 on
Linux aarch64. Every Cargo leaf reported the same cache identity:

```text
family: cargo-target
scope: slot
digest: dd27515db6a0fc48aabe31c5e300af7325e3af24a98f0560364fa9c516971e03
```

### Parent and candidate measurements

The Phase 05 raw-target candidate is the direct parent for this scope change.
The cache identity is expected to change placement and lifecycle ownership,
not compile policy or artifact format.

| State | Full CI | Nextest total | Nextest execution | Cargo cache | files | `.dwo` files | trybuild |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Phase 05 raw target, clean | 307.13s |  — | — | 9.8 GiB | 17,791 | 0 | 1.7 GiB |
| Phase 05 raw target, warm | 197.49s | — | — | 9.8 GiB | 17,791 | 0 | 1.7 GiB |
| Phase 06 cache, clean | 322.37s | 147.891s | 105.561s | 9.8 GiB | 17,790 | 0 | 1.7 GiB |
| Phase 06 cache, warm | 201.83s | 95.412s | 75.643s | 9.8 GiB | 17,790 | 0 | 1.7 GiB |

The clean Phase 06 cache occupied 10,422,096,724 bytes and 20,930 entries;
the warm repeat had the same size and entry count. The empty incremental
directories remained 4 KiB placeholders, with no incremental artifacts. The
trybuild subtree occupied 1,761,961,195 bytes. The derived compile/link and
process remainder was approximately 42.33s clean and 19.77s warm after
subtracting the Nextest execution summaries; these are task-level residuals,
not an independently instrumented linker timer.

The clean and warm full-CI runs both passed 13/13:

- clean: `run-4155691-1784123066632728835`, 322.37s;
- warm: `run-4192934-1784123414461861861`, 201.83s.

The clean Nextest run passed 974/974 tests. Doctests remained 53 binaries and
42 doctests; trybuild remained 9 harnesses, 80 UI cases, and 71 checked
stderr baselines; database/parity coverage remained 6 leaves; and CI remained
13 tasks. The clean and warm doctest tasks took 16.247s and 14.041s.

### Isolation, invalidation, and cleanup

The same digest was materialized at distinct paths for slot 7 and slot 8.
Two concurrently launched `cargo-metadata-contract` tasks passed without
cross-contamination:

- slot 7: `run-12541-1784123817324835829`, 33.936s;
- slot 8: `run-14329-1784123821352459016`, 33.568s.

The same slot/digest combination under a separate
`NIXFIED_STATE_DIR` also materialized beneath that separate state root:
`run-11756-1784123637754781294`. The slot-8 focused proof was
`run-11700-1784123629592338163`.

Changing only the policy key produced digest
`9e552086a3759121242668819fd47c6c5604339b100e888bfe7f309a7c4b91dd` in
`run-11946-1784123677805871166`. Changing only the declared toolchain key
produced digest
`23d5bdddaaec9f53636136bbc0429a36ae6aa6ec1301e63e6867b0a66548c9ca` in
`run-12058-1784123689352842570`. Platform identity is included in the key and
the runtime target identity, but cross-platform execution remains unverified.

For the owner-approved manual cleanup check, only the exact reported Phase 06
digest directory was removed. `pgdata` remained at 1,025 entries, run-record
state remained at 111 entries, and the Cargo cache family went from 20,931
entries to zero. No service, registry, or diagnostic parent was deleted.

### Phase 06 acceptance

All applicable checks pass under the explicit re-scope: unchanged sequential
runs reused one identity; slots and separate state roots isolated their paths;
policy and declared toolchain changes invalidated the identity; the two-slot
concurrent focused run passed; and exact-path cleanup preserved non-cache
state. Disk growth was flat across the unchanged clean/warm pair. The
remaining limitation is that cleanup is manual for this pilot because the
locked Nixfied runtime still has no first-class cache-family cleanup surface;
that limitation is tracked in `docs/nixfied-capability-gaps.md`.

Before the Phase 06 commit, the required gates passed on the dirty candidate:

- `nix run .#check -- --slot 7`: 4/4 in 37.86s,
  `run-24675-1784123910494494304`;
- `nix run .#test -- --slot 7`: 2/2 in 170.09s,
  `run-30503-1784123953004592181`;
- `nix run .#test-db -- --slot 7`: 6/6 in 108.60s,
  `run-54899-1784124126546367061`; and
- `nix run .#ci -- --slot 7`: 13/13 in 216.93s,
  `run-57359-1784124239044961725`.

The clean committed-tree evaluation of Phase 06 then passed model admission
with the same model hash and full CI 13/13 in both cache states:

- clean exact cache: 333.82s,
  `run-70857-1784124512678583396`;
- warm exact cache: 231.82s,
  `run-108931-1784124855072711127`.

The clean committed samples are slower than the dirty-candidate samples by
11.45s and 29.99s respectively, which is retained as VM scheduling/service
variance rather than attributed to the cache identity change. No source or
model behavior changed between those evaluations.

The final warm verification at the pre-measurement-amend committed revision
also passed 13/13 in 213.81s under
`run-122464-1784125134265099051`; the subsequent amendment adds measurement
text only and does not alter the model or task behavior.

The final `.#ci` program was 327 bytes. Its realized Nix closure remained 124
paths and 12,219,706,744 NAR bytes, with the model/toolchain closure accounting
for the same approximately 1.59 GiB store output observed in Phase 05.
