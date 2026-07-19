# Controlled slow-build baseline

Status: historical measurements through Revision 2 R2-11; current
responsibility correction recorded in the final section

The timings, sizes, inventories, hashes, and failure observations below are
preserved as measured. References to Nixfied cache identities and lifecycle
requests describe the experimental configuration at that time, not the current
architecture. Broad gates now use project-owned `target/verification`.

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
the later, now-superseded handoff recorded that historical limitation.

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

## Phase 07: measured verification profile experiment

Phase 07 was measured on 2026-07-15 on Linux aarch64 with Rust/Cargo 1.96.0,
starting from committed Phase 06 revision
`87487abb3d53055f8384ef5c150d69f958d76913`. The baseline retained the Phase 06
verification policy: nonincremental builds, `debug=1`, and split debuginfo off.
Each variant used its own disposable `NIXFIED_STATE_DIR`; no variant reused
another variant's Cargo artifacts. The runtime-reported cache digest happened
to remain `dd27515db6a0fc48aabe31c5e300af7325e3af24a98f0560364fa9c516971e03`
because the Phase 06 cache key was intentionally unchanged.

### Isolated variants

| Variant | Temporary profile setting | Model hash | State root |
| --- | --- | --- | --- |
| Default | no additional profile setting | `d25c647af500ec8c060e173f05e199564cf3d3921663c82f04c3477a4c49ce84` | `/tmp/mfm-phase07-default` |
| `codegen-units=256` | dev/test codegen units `256` | `6145b87382af1d29ebf24b666e3b7ce825a117aed4a99d9020759474f21788fd` | `/tmp/mfm-phase07-cgu256` |
| `opt-level=1` | dev/test opt level `1` | `bd1c553808119f3ee0e2d08e3ce937342fe6d3d1002d574627301e44bb7d1ee8` | `/tmp/mfm-phase07-opt1` |

The two non-default settings were temporary measurement edits to
`nixfied.nix` and were removed before the Phase 07 report commit.

### Full-CI and focused leaf measurements

Each full run passed all 13 CI tasks. Nextest passed 974/974 tests in every
run. `Nextest compile` is Cargo's logged `Finished` interval; `Nextest
execution` is the Nextest summary interval. Linking is included in Cargo's
compile interval because the task graph does not expose a separate linker
timer; the remaining task interval after those two values was 0.38--0.60s.
Parity is the sum of the seven parity leaf durations, not their concurrent
critical path.

| Variant | State | Run | Full CI | Nextest task | Compile | Execution | Doctest task | Parity sum |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Default | clean | `run-136609-1784125579154346098` | 341.392s | 160.765s | 45.93s | 114.415s | 15.903s | 95.368s |
| Default | warm | `run-173731-1784125927514157988` | 226.593s | 110.181s | 25.10s | 84.651s | 16.399s | 78.136s |
| `codegen-units=256` | clean | `run-186931-1784126306418770008` | 343.383s | 154.753s | 51.45s | 102.923s | 22.103s | 95.512s |
| `codegen-units=256` | warm | `run-262519-1784126655476140855` | 243.883s | 114.991s | 30.31s | 84.078s | 18.850s | 85.271s |
| `opt-level=1` | clean | `run-285647-1784126929587222287` | 470.965s | 233.171s | 115s | 117.608s | 10.669s | 81.149s |
| `opt-level=1` | warm | `run-323368-1784127408590034967` | 236.160s | 140.381s | 40.57s | 99.214s | 11.719s | 55.931s |

The focused one-leaf-edit runs added and then removed a comment in
`crates/kernel/program/src/lib.rs`; each still passed 974/974 tests:

| Variant | Run | Focused Nextest task | Execution summary |
| --- | --- | ---: | ---: |
| Default | `run-375417-1784128068095169401` | 142.090s | 94.759s |
| `codegen-units=256` | `run-352545-1784127904596935915` | 147.178s | 84.379s |
| `opt-level=1` | `run-337222-1784127682589024154` | 203.552s | 102.749s |

The check-leaf durations were also captured from the full runs. In the order
`fmt / clippy / cargo-metadata-contract / SQLx-offline`, they were:

| Variant | Clean | Warm |
| --- | --- | --- |
| Default | 1.035 / 20.785 / 20.310 / 6.944s | 1.131 / 4.382 / 1.582 / 0.149s |
| `codegen-units=256` | 1.063 / 22.601 / 23.667 / 5.701s | 1.052 / 3.802 / 1.123 / 0.147s |
| `opt-level=1` | 1.422 / 29.657 / 72.546 / 7.361s | 1.382 / 5.087 / 2.266 / 0.235s |

### Artifact and trybuild measurements

The cache sizes below were captured after each variant's clean and warm full
runs, before the separate leaf-edit probe. All variants produced zero `.dwo`
files and no incremental artifacts; the empty incremental directory remained
only as a filesystem placeholder.

| Variant | Cache bytes | Human size | Files | Entries | Trybuild bytes |
| --- | ---: | ---: | ---: | ---: | ---: |
| Default | 10,421,978,299 | 9.8G | 17,790 | 20,930 | 1,761,909,858 |
| `codegen-units=256` | 10,257,175,248 | 9.7G | 17,601 | 20,715 | 1,777,342,998 |
| `opt-level=1` | 9,102,066,063 | 8.6G | 16,246 | 19,188 | 1,792,098,139 |

Trybuild remained nested inside the workspace test/doctest tasks rather than a
separate Nixfied task. Clean logs show `trybuild v1.0.115` compilation in the
affected task; warm logs omit that compilation. The retained inventory is nine
harnesses, 80 Rust UI cases, and 71 checked stderr baselines. Its execution
time is therefore included in the Nextest/doctest execution intervals above,
while its artifact footprint is reported separately.

### Correctness and diagnostics

The actual `opt-level=1` compiler command contained `-C opt-level=1`,
`-C debuginfo=1`, and `-C debug-assertions=on`. The default compiler command
used no explicit optimization override, so it retained Cargo's test-profile
level 0, and emitted `debuginfo=1`. `rustc --test --print cfg` reported
`debug_assertions`.

A disposable profile probe ran under both default settings and `opt-level=1`.
It asserted `cfg!(debug_assertions)` and caught a runtime `u8` overflow panic;
both probes passed 1/1, confirming debug assertions and overflow checks stayed
enabled in the tested profiles. The focused Linux auto-lock diagnostic test
also passed 1/1 in 0.83s with `RUST_BACKTRACE=1`. Its test binary contained
`.debug_info`, `.debug_line`, and decoded entries for
`crates/core/src/keystore/filesystem_tests.rs`, preserving source-line
diagnostics. No panic was generated by the passing test, so no new failure
trace was available. macOS was unavailable and remains unverified.

### Phase 07 acceptance and decision

The default profile remains the winner. Relative to it, `codegen-units=256`
was 0.58% slower clean, 7.63% slower warm, and 3.58% slower after the leaf
edit. `opt-level=1` was 37.95% slower clean, 4.22% slower warm, and 43.26%
slower after the leaf edit. Both alternatives passed correctness, but neither
improved total gate behavior; `opt-level=1` also materially increased clean
compile time. No profile tuning is retained.

Phase 07 is therefore a report-only rejection. The only committed change is
this measurement record, with subject `docs: record verification profile
experiment`; the Phase 06 default profile remains frozen for both cache pilots.
The Linux matrix is complete for the requested variants and cache states;
cross-platform consistency remains an explicit macOS limitation.

### Clean committed-tree confirmation

After the report-only commit, the exact committed tree was re-evaluated with
model admission and the default Phase 06 profile. Model admission remained
`d25c647af500ec8c060e173f05e199564cf3d3921663c82f04c3477a4c49ce84`, with the
same runtime ABI and toolchain. The clean committed run passed 13/13 in
436.377s under `run-453832-1784129299694723069`; the warm committed run passed
13/13 in 249.277s under `run-498516-1784129741812724352`. Both committed runs
passed 974/974 Nextest tests. The committed cache after the pair was
10,421,992,215 bytes, 20,930 entries, 17,790 files, zero `.dwo` files, and
1,761,915,884 bytes in trybuild artifacts. This confirms that the report-only
commit did not alter the frozen verification profile or task graph.

## R2-01: refreshed workload and gate baseline

R2-01 was measured locally on 2026-07-17 at exact clean revision
`6f7146e449f5a5d26f5666db264cd91593d7b2ba` (`docs: rfc builds tt6`). This
report edit was not present in the measured tree. No cache technology, task,
feature, profile, service, workflow, or test selection changed during the
measurements.

### Reference host and virtualization manifest

- Guest OS: Ubuntu 24.04.4 LTS, Linux `6.8.0-124-generic`, `aarch64`.
- Virtualization: a Lima guest on Apple's Virtualization framework. The guest
  reports `systemd-detect-virt=apple`, DMI product `Apple Virtualization
  Generic Platform`, vendor `Apple Inc.`, and hostname `lima-devvm`.
- CPU: 8 online aarch64 CPUs, one thread per core and one 8-core cluster. The
  guest exposes Apple as the CPU vendor but no model identifier or frequency
  governor.
- Memory: 16,732,610,560 bytes of guest RAM and 8,589,930,496 bytes of swap.
- Storage: a 200 GiB virtio disk with a 199 GiB ext4 root partition mounted
  read-write with `relatime`, `discard`, `errors=remount-ro`, and `commit=30`.
  The workspace and all measurement roots were on that filesystem. It had
  193 GiB formatted capacity and 86 GiB available before the full-CI matrix.
- Load controls: all timed gates ran serially, with no intentional competing
  build, no compiler wrapper, empty `RUSTC_WRAPPER`, and no `sccache` binary
  on `PATH`. No sample was discarded. The VM remained subject to normal host
  scheduling, and the warm results retain one visible scheduling outlier.
- Scope: all performance evidence came from this local Linux guest. No hosted
  workflow was dispatched and no macOS timing was collected. The existing
  hosted Linux/macOS workflow was inspected only as a correctness and
  portability contract.

The pinned identities were:

- Rust `1.96.0` (`ac68faa20c58cbccd01ee7208bf3b6e93a7d7f96`) and Cargo
  `1.96.0` (`30a34c6821b57de0aaec83a901aca39f88f6778c`), target
  `aarch64-unknown-linux-gnu`, LLVM 22.1.2;
- Cargo Nextest 0.9.137 and SQLx CLI 0.9.0;
- Nix 2.34.7 and nixpkgs revision
  `3e41b24abd260e8f71dbe2f5737d24122f972158`;
- Nixfied revision `b0681e45ab76d5023d9c5e033087d34adf98e90b`, runtime ABI
  `nixfied-runtime-abi:1-5ff3aa14f2bf`, and model hash
  `d25c647af500ec8c060e173f05e199564cf3d3921663c82f04c3477a4c49ce84`;
  and
- Rust overlay revision `fe64e6b409dc513274d2941f8da13bbd0fdcf44e`.

### Frozen workload inventory

Cargo metadata reported 53 workspace packages and 100 targets. The admitted
verification surface was:

- 974 Nextest identifiers across 99 binaries. Every measured full gate passed
  974/974. Sorting identifiers as `<binary-id>::<test-name>`, one per line in
  byte order with a trailing newline, produced SHA-256
  `e75d6ea039b5507c6f9b89bef89656e31073c02f8f17f74f680fc2bbf0d67f08`.
- 53 doctest binaries and 42 passing doctests.
- Nine trybuild harnesses, 80 Rust UI sources, and 71 checked `.stderr`
  baselines. Trybuild remains inside the Nextest surface rather than being a
  separate Nixfied leaf.
- Two SQLx query-macro sites and two checked `.sqlx` metadata files. The
  offline SQLx leaf and the online disposable-schema mutation/recovery leaf
  both passed in every applicable gate.
- Nine tests in the Cargo metadata and architecture contract binary.
- Six `test-db` leaves: online SQLx, CLI build, state-event parity,
  collect/report parity, REST parity, and CLI/Postgres status parity. The five
  direct parity test leaves outside Nextest passed 4, 1, 3, 1, and 1 tests
  respectively when including the separate keystore leaf.
- Thirteen expanded CI leaves: four `check`, two `test`, one keystore parity,
  and six `test-db` leaves.

Declared features remained exactly:

| Package | Declared features |
| --- | --- |
| `mfm` | `parity-tests` |
| `mfm-app` | `test-support` |
| `mfm-integration-tests` | `parity-tests` |
| `mfm-rest-api` | `parity-tests` |
| `mfm-store` | `test-support` |
| `mfm-stream-store-postgres` | `parity-tests` |
| `mfm_core` | `default`, `dangerous-secret-export` |

The active gate selections also remained frozen: Clippy uses `--all-features`,
workspace Nextest enables `mfm-app/test-support`, and SQLx/parity leaves enable
their package's `parity-tests` feature. Exact identifiers and selections, not
only aggregate counts, are the comparison contract for later experiments.

### Developer and flake observations

One persistent `nix develop` command exposed the pinned tools, an empty
`CARGO_TARGET_DIR`, and Cargo's worktree-owned developer target. On the
initial state encountered for this revision, `cargo check -p mfm-program`
took 3.56s and `cargo test -p mfm-program --lib` took 5.99s and passed 34
tests. These are named developer observations, not a clean/warm matrix.

`nix run .#quick` passed its format and broad workspace-check contract in
19.42s, including a 17.11s Cargo check. `nix flake check -L` passed in 2.13s.
It evaluated the `mfm` Rust package derivation and every other package,
development-shell, and app output, but did not realize the `mfm` derivation or
run Rust compilation. It is therefore flake/schema admission, not a duplicate
Rust test gate in the current flake.

### Admitted task graph and public-gate equivalence

Model admission reported 21 total task definitions. The public composites
expand as follows:

| Public verb | Admitted leaves | Ordering |
| --- | --- | --- |
| `.#check` | format, Clippy, Cargo metadata contract, offline SQLx | serial in that order |
| `.#test` | Nextest, doctests | serial in that order |
| `.#test-db` | online SQLx, CLI build, four Postgres parity leaves | SQLx first; build and state-event parity may follow; remaining parity chain is ordered |
| `.#ci` | the same `check`, `test`, and `test-db` composites plus keystore parity | `check -> test -> keystore parity -> test-db` |

The nested composites use the same task definitions, arguments, environments,
features, services, cache identity, and ordering semantics as the component
verbs. Run-once behavior applies inside one composite invocation only; it does
not reuse a success from a previous public-gate invocation.

### Full-gate matrix

Three fresh `NIXFIED_STATE_DIR` roots supplied independent clean Cargo target
caches. Each root used slot 0 and the runtime-reported cache digest
`dd27515db6a0fc48aabe31c5e300af7325e3af24a98f0560364fa9c516971e03`.
The five warm runs reused those exact roots and digests; three reused root C.
All runs were serial and passed 13/13.

`Nextest compile` is Cargo's logged `Finished` interval and includes linking.
`Nextest execution` is Nextest's summary interval. `Verification tail` is the
sum of the seven post-test leaves from keystore parity through the database
chain; because this graph is serial, it is also their contribution to the
critical path, apart from service preparation and task-launch overhead.

| State | Run | Full CI | Nextest task | Compile | Execution | Doctests | Verification tail |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Clean A | `run-2935615-1784289015199459752` | 331.422s | 150.059s | 48.38s | 101.273s | 16.728s | 111.501s |
| Clean B | `run-2984572-1784289578082224200` | 328.831s | 153.259s | 47.07s | 105.797s | 18.300s | 108.551s |
| Clean C | `run-3039714-1784290140624314687` | 337.111s | 150.248s | 48.05s | 101.740s | 16.620s | 119.514s |
| Warm A1 | `run-2971979-1784289351883826711` | 218.067s | 103.845s | 25.47s | 77.947s | 14.640s | 91.960s |
| Warm B1 | `run-3027085-1784289917897436715` | 214.081s | 100.855s | 18.51s | 81.971s | 16.320s | 90.064s |
| Warm C1 | `run-3084720-1784290493881589189` | 258.147s | 138.390s | 41.19s | 96.427s | 15.320s | 90.627s |
| Warm C2 | `run-3099679-1784290762518379370` | 211.892s | 99.213s | 20.62s | 78.137s | 15.250s | 89.713s |
| Warm C3 | `run-3112269-1784290979166705531` | 203.752s | 95.661s | 20.22s | 75.094s | 14.380s | 87.106s |

The clean median was 331.422s and the warm median was 214.081s. The 117.341s
clean-to-warm difference is 35.4% of the clean median. The clean Nextest
median decomposed into 48.05s of compile/link and 101.740s of execution; the
warm medians were 20.62s and 78.137s. Warm C1 is retained as an uncontended
host-scheduling outlier rather than discarded. Even outside that outlier,
execution and the service/parity tail dominate the residual warm gate.

Each untouched full-CI cache occupied 10,421,922,551 bytes (9.8 GiB), 17,790
files and 20,931 total entries, including 1,761,885,682 bytes of nested
trybuild artifacts. There were no `.dwo` files or incremental artifacts. No
compiler-object cache was enabled.

### Local contribution-workflow cost

The component gates and an immediate CI rerun used the same revision, slot,
state root, cache digest, and serial host controls:

| Invocation | Gate result | Gate time | Outer wall time |
| --- | --- | ---: | ---: |
| `nix run .#check` | 4/4 | 10.73s | 11.77s |
| `nix run .#test` | 2/2 | 118.23s | 118.95s |
| `nix run .#test-db` | 6/6 | 92.62s | 93.92s |
| immediate `nix run .#ci` | 13/13 | 254.22s | 254.45s |

The component sequence cost 224.64s of outer wall time. CI then reran every
component leaf and added 254.45s, making the observed contribution workflow
479.09s. The only coverage added by CI beyond those three component verbs was
keystore parity, which took 1.34s in the CI rerun. The rerun is 53.1% of the
total observed workflow. This is evidence that gate governance is eligible
for a later, separately approved policy proposal; R2-01 changes no gate or
required contributor policy.

### R2-01 decision and limitations

The canonical workloads are frozen at the exact identifiers, feature
selections, task graph, and service semantics above. The target-only design
remains the control for later candidates. Gate-governance evaluation is
eligible because sequential component gates followed by CI repeat nearly the
entire assurance surface at material cost, but the keystore-only CI coverage
must be preserved by any later proposal.

The measurements are Linux/aarch64 observations from one Lima VM. They do not
claim measured macOS or hosted-runner performance. The guest does not expose
the physical Apple CPU model or a guest frequency governor, and normal host
scheduling produced one retained warm outlier. Service startup is included in
full wall time but is not independently timestamped by the current runtime;
the task-level compilation, execution, doctest, and verification-tail
decomposition is the available evidence. No R2-01 implementation behavior is
left unverified because the phase is report-only.

## R2-02: developer build-boundary experiment

R2-02 was measured on the R2-01 reference host on 2026-07-17 from committed
revision `a4db7d98892a7bf2e2fdef44a014cf4b266b606c` (`docs: refresh build
workload baseline`). All probes used the pinned Rust/Cargo 1.96.0 development
shell. The only source mutations were comment-only edits in detached
disposable worktrees; their original SHA-256 digest was restored before
repository verification. No Cargo configuration, flake, Nixfied model,
public gate, or `.#quick` behavior changed.

### Direct Cargo and `.#quick`

Detached worktree A began with no `target` directory. One persistent
`nix develop` command reported an empty `CARGO_TARGET_DIR` and ran the focused,
reverse-dependent, broad, and quick checks serially:

| Developer feedback surface | Clean/first | Warm |
| --- | ---: | ---: |
| `cargo check -p mfm-program` | 3.81s | 0.09s |
| `cargo test -p mfm-program --lib` | 6.25s | 0.09s |
| `cargo check -p mfm-certify` | 0.73s | 0.06s |
| `cargo check --workspace --lib --bins` | 17.04s | 1.08s |
| warm `nix run .#quick` | not applicable | 2.37s |

The focused test passed 34/34. The warm quick interval includes Nix app
startup, format verification, and the same broad Cargo check. It is useful as
a preflight convenience, but invoking it for every edit is slower than direct
focused Cargo because its contract is deliberately broader.

A comment-only edit to `mfm-program` changed the source digest from
`9a83e399172bb8a3457b36414ffd5079ff1455f47669e50e1c1295d7f0844446` to
`d833dfaafc99a764f2b0e05c7aa7151aee449ba3edbe9b15cd1504532f9723a6`.
The edited focused check took 0.26s, the reverse-dependent check 0.20s, and
`.#quick` 3.52s. Restoring the original digest took 0.24s focused and 2.89s
through quick. Cargo retained both variants without stale success.

The worktree-local target grew from 1.6 GiB/7,279 files after the initial
matrix to 1.8 GiB/7,432 files after the edit cycle. It contained normal
developer incremental and split-debug artifacts. Detached worktree B still
had no target at that point; its first identical focused check independently
took 3.48s, its warm repeat took 0.07s, and its new target occupied 131 MiB
and 358 files. The targets resolved to distinct paths under their respective
worktrees and shared no mutable artifact state.

### Stable `build-dir` separation

The stable Cargo `CARGO_BUILD_BUILD_DIR` surface was tested with one shared
build directory and distinct target directories. This is a compatibility
probe, not an endorsed cross-workspace configuration.

| Probe | Wall time | Observation |
| --- | ---: | --- |
| Seed focused check | 3.88s | populated the shared build directory |
| Same target, warm | 0.10s | normal fingerprint reuse |
| Distinct target, same worktree | 0.07s | reused shared intermediates |
| Identical relocated worktree | 0.07s | reused across source paths |
| Relocated comment edit | 0.59s | rebuilt the changed leaf |
| Original source after edited variant | 0.07s | both variants coexisted |

After the focused relocation matrix, the shared directory occupied 142 MiB
and 361 files while each target directory occupied only 8 KiB. Two new
comment variants then wrote concurrently from the two worktrees. Both checks
passed in 0.40s/0.39s; Cargo logged waits on its package-cache locks and one
wait on the build-directory lock. The shared directory did not corrupt, but
the writers serialized rather than providing independent progress.

The `mfm-program` trybuild harness also passed through the separated layout:
all 26 UI cases passed, including 21 checked compile-fail expectations. The
outer test binary and most intermediates landed in the shared build directory,
which grew to 1.2 GiB, while trybuild still created its nested target structure
under the caller's target directory. The complete harness took 21.60s.

### Rust-analyzer and nightly observations

Direct Cargo and rust-analyzer were launched concurrently after another
comment-only edit. Cargo passed in 0.32s without target corruption. The shell,
however, contains neither a pinned rust-analyzer executable nor `rust-src`.
The `rust-analyzer` selected from the ambient rustup proxy matched version
1.96.0 but failed to execute the Nix-store rustc after sanitizing its child
environment (`libz.so.1` was unavailable). It returned status zero only after
falling back to incomplete metadata: zero dependency lines and 414/414 failed
constant evaluations.

Invoking the underlying rustup rust-analyzer binary directly avoided that
proxy failure and loaded 2,645,743 dependency lines, but it still reported
missing standard-library sources and 1,190 failed target data-layout queries
(74%). That diagnostic run took 21.21s. Process coexistence therefore did
not damage Cargo state, but the current development shell does not provide a
complete, pinned IDE contract. Ambient tooling is not accepted as a hidden
developer-lane dependency.

Pinned stable Cargo lists `-Z build-dir-new-layout` in its unstable options but
rejects the flag because it is not a nightly channel. No nightly toolchain was
installed on the reference guest. R2-02 records the new layout only as an
upstream compatibility observation and does not bypass the channel check or
introduce an unpinned toolchain.

### Cleanup ownership

Normal worktree-local cleanup was correctly scoped. `cargo clean` in worktree
A removed 8,091 files and its 2,137,414,680-byte target while leaving
worktree B's 135,197,890-byte target unchanged.

Shared-build cleanup had a materially different ownership boundary. Running
`cargo clean` as one client removed 4,313 files and the entire
1,216,114,352-byte shared build directory. Other clients' 8 KiB target
directories remained but no longer had reusable intermediates; the relocated
client recovered correctly with a 3.95s cold rebuild. Any sharer can therefore
invalidate every other sharer, and there is no supported repository/worktree
owner, retention rule, or stale-unit lifecycle for this arrangement.

### R2-02 decision

The current two-lane artifact boundary is confirmed for the architecture
comparison: persistent `nix develop` plus direct, worktree-local Cargo is the
primary edit loop; `.#quick` remains a broader convenience; and mutable
developer targets remain separate from verification state and from other
worktrees.

Cross-workspace `build-dir` sharing is rejected for adoption despite its
strong focused reuse. It may be retested only after Cargo officially supports
cross-workspace sharing with the new content-scoped layout, granular
concurrency, automatic stale-unit cleanup, and a documented ownership and
compatibility contract. MFM must not grow a project-local manager around an
unsupported Cargo cache.

The final architecture comparison also carries one project-local developer
tooling prerequisite: the pinned development environment must provide a
compatible rust-analyzer and `rust-src` before IDE coexistence can be claimed
as supported. That follow-up is distinct from changing the artifact boundary
and is not implemented in this report-only phase. Linux/aarch64 is the only
measured platform; no hosted or macOS performance sample was used.

## R2-03: verification execution-topology experiment

R2-03 was measured on 2026-07-17 from committed R2-02 revision
`449b828e9c8a2945cf139a65d2daa6407ad8494b`. It used the R2-01 reference host,
the pinned Rust/Cargo 1.96.0 shell, slot 1 under the isolated
`/tmp/mfm-r2-01-b` state root, and the existing verification cache identities.
The accepted samples were precompiled or immediately prewarmed; prewarm time
is reported separately and is not counted as an execution improvement.

One disposable `r2-03-test-db` composite was admitted only for the database
topology probe. It was never added to the public verbs and was removed after
the run. No authoritative task, dependency, feature selection, partition,
worker count, service, or gate changed.

### Nextest worker screening

Each accepted run used the authoritative
`cargo nextest run --workspace --features mfm-app/test-support` selection and
passed the exact 974-test, 99-binary inventory. The host exposes eight logical
CPUs. An immediate `--no-run` invocation normalized the selected artifacts
before each timed execution.

| Workers | Prewarm | Nextest execution | Outer wall | Result |
| ---: | ---: | ---: | ---: | --- |
| 1 | 20.06s | 195.512s | 196.16s | 974/974 passed |
| 4 | 21.08s | 77.498s | 78.35s | 974/974 passed |
| 8 | 0.57s | 78.762s | 79.26s | 974/974 passed |
| 16 | 1.01s | 91.415s | 92.21s | 974/974 passed |

Four workers were the screening minimum, but only by 1.264s, or 1.6%, against
eight workers. Sixteen workers oversubscribed the guest and regressed 16.1%
against eight. The 4/8-worker result also agrees with R2-01's 78.137s warm
execution median; it does not justify freezing a host-specific worker count.
One earlier one-worker observation was discarded because another full CI run
was active. A malformed `--all-features` probe was also discarded immediately:
it selected 1,018 rather than 974 tests and encountered an invalid temporary
directory. Neither discarded interval appears in the table.

Count partitions were not run after worker screening. They would start
multiple Cargo/Nextest processes against the same target lock, or require
separate targets and duplicated compilation. Nextest already distributes the
99 binaries while the checked-in test group applies the narrower constraint
that matters: one Trybuild UI harness at a time.

### Trybuild and Cargo-lock boundary

The nine UI harnesses remain in the `trybuild-ui` group with `max-threads=1`.
The shared nested Trybuild target occupies 1.7 GiB. It contains shared
`debug/deps` state and one generated project per harness, so concurrently
running the harnesses in separate processes against that directory would
still contend on one Cargo lock.

Per-harness target isolation was screened as the safe alternative. The guest
filesystem does not support copy-on-write reflinks: an attempted disposable
clone failed closed with `Operation not supported` and created no copied
files. Nine physical copies would require approximately 15.3 GiB before any
relocation-triggered rebuild, on a filesystem that had approximately 33 GiB
available at the end of the experiment. Such copies would make apparent
execution overlap by duplicating compilation and cache ownership. The probe
therefore stopped rather than presenting that setup cost as a test-execution
gain, and its empty disposable directories were removed.

The current serialized group is consequently the only validated single-target
policy. The 8- and 16-worker tails show that more outer concurrency does not
accelerate these nested-Cargo suites. A future proposal would need a first-class
precompiled UI artifact or independently owned nested targets with measured
build and retention cost; removing the group or launching partitions against
the shared target is not safe.

### Doctest overlap

A standalone doctest command passed all 53 doctest binaries and 42 discovered
tests. Its first direct interval was 32.49s because it changed Cargo
fingerprints; the warm concurrent interval was 15.42s. That first interval is
setup evidence, not part of the overlap comparison.

After a 0.57s Nextest prewarm, four-worker Nextest and warm doctests ran
concurrently against the same target and both passed. Their joint wall time
was 89.131s. Nextest itself grew from 78.35s to 89.12s, while the comparable
warm serial sum is 93.77s. The overlap therefore saved only 4.64s, or 4.9%,
and delayed the binary-test result by 10.77s. Doctest overlap is rejected.

### Postgres and parity topology

Managed Postgres selected `127.0.0.1:28180` in slot 1. A warm persistent start
plus readiness and migration took 0.698s end to end; the migration task itself
took 0.031s. The online SQLx task already creates a process-unique schema,
migrates it, deliberately corrupts it, proves rejection, recreates it, proves
acceptance, and drops it. Each state-store, collect/report, REST, and CLI
status test also creates a unique schema and drops it. The four parity test
binaries are therefore database-isolated from one another; they share only
the managed server, service port, and prebuilt CLI artifact where applicable.

The warm authoritative CI run
`run-3295302-1784294297744517743` showed the current database chain:

| Leaf | Duration |
| --- | ---: |
| online SQLx | 4.631s |
| CLI build | 7.323s |
| state/store events | 6.719s |
| collect then report | 47.685s |
| REST smoke | 4.568s |
| CLI status | 12.372s |

These leaves contribute approximately 83.30s serially. Although the admitted
graph can express independent branches, every Cargo leaf leases the same
slot-scoped `CARGO_TARGET_DIR`. The runtime correctly prevents concurrent
writers, so graph edges alone cannot produce Cargo execution overlap.

The disposable graph proved that constraint. It kept online SQLx first, made
CLI build and schema-isolated parity branches eligible as soon as their true
inputs existed, and passed all six leaves under
`run-3442964-1784296496893010769`. It nevertheless took 106.428s. Completion
timestamps were serial, and its leaf durations were 7.659 / 14.891 / 17.747 /
51.625 / 5.059 / 8.531s for SQLx, build, status, collect/report, REST, and
state/store respectively. The shared cache lease preserved correctness but
made the nominal parallel graph 27.8% slower than the warm authoritative
chain.

To separate compilation and cache locking from true test execution, the four
already-built parity binaries were then run against the managed server. The
serial execution-only durations were 3.11s state/store, 0.24s REST, 6.12s CLI
status, and 40.69s collect/report: 50.16s total. Bounded concurrent execution
passed the same 43 tests in 41.109s wall time; the leaves took 3.77 / 0.51 /
6.84 / 41.10s. This is a 9.05s, 18.0% execution-only reduction, but it is less
than the RFC's required 30-second end-to-end materiality floor and is not
implementable merely by removing graph dependencies. A later execution-runner
design could reconsider precompiled parity binaries, but R2-03 creates no
implementation proposal.

### Resources, diagnostics, and decision

The reference guest had eight logical CPUs, 15.6 GiB RAM with approximately
14.4 GiB available after the probes, and 8 GiB swap with 0.55 GiB in use. The
filesystem had 35.4 GB available at the final sample. The normal database
target remained 9.8 GiB; repeated feature-screening artifacts grew the
disposable Nextest target to 11 GiB, reinforcing why duplicate targets are
not a free execution mechanism. Slot-derived port 28180 and unique schemas
provided service isolation. Every accepted sample retained ordinary per-task
stdout/stderr, exit status, exact test counts, and source-line backtraces.
Coverage drift, a missing temporary directory, unsupported reflinks, and the
shared-cache lease all produced explicit diagnostics rather than silent
fallbacks.

No R2-03 topology qualifies: four workers save only 1.6%/1.264s versus eight,
doctest overlap saves 4.9%/4.64s, and parity execution overlap saves
18.0%/9.05s only after excluding Cargo/setup. Each misses the requirement to
improve an end-to-end target by both 15% and 30 seconds. The current serial
authoritative graph and default Nextest scheduling remain unchanged.

Compilation also remains a bounded residual rather than the whole gate. From
R2-01, the warm median logged Nextest compile/link interval is 20.62s, only
9.6% of the 214.081s full-CI median; the clean interval is 48.05s, 14.5% of
331.422s. Eliminating either interval entirely would still leave the dominant
test and parity execution floor. R2-04 should therefore characterize the
durable Cargo verification target without assuming execution-topology changes
will amplify its result.

## R2-04: persistent local Cargo verification experiment

R2-04 was measured on 2026-07-17 from committed R2-03 revision
`6507999cfad64c4b953184cb2998db4074f04452`. It retained the authoritative
compact verification profile, Rust/Cargo 1.96.0, Nixfied model hash
`d25c647af500ec8c060e173f05e199564cf3d3921663c82f04c3477a4c49ce84`,
runtime ABI `nixfied-runtime-abi:1-5ff3aa14f2bf`, and cache digest
`dd27515db6a0fc48aabe31c5e300af7325e3af24a98f0560364fa9c516971e03`.
No compiler wrapper or Nix-built Rust artifact was added.

The primary experimental state root was
`/tmp/mfm-r2-04-main-state`. A detached worktree at
`/tmp/mfm-r2-04-wt` contained every tracked-file mutation. The authoritative
worktree remained unchanged until this report was written. Each mutation's
original digest was restored and the detached worktree returned clean before
the next case.

### Clean and unchanged control

The new clean and warm pair supplements, rather than replaces, R2-01's three
clean and five warm full-CI samples. Both R2-04 runs passed all 13 leaves,
974/974 Nextest tests, 53 doctest binaries with 42 tests, nine Trybuild
harnesses, online/offline SQLx, keystore, CLI, REST, and Postgres parity.

| State | Run | Full CI | Outer wall | Nextest task | Compile/link | Execution |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| clean target | `run-3492136-1784300829850094755` | 304.51s | 304.72s | 137.602s | 40.67s | 96.569s |
| exact unchanged | `run-3528870-1784301171449762270` | 204.01s | 204.25s | 94.211s | 19.09s | 74.735s |

The unchanged durable target saved 100.50s and 33.0%. R2-01's larger matrix
reported a 117.341s/35.4% clean-to-warm median improvement. Both independently
clear the RFC's 15% and 30-second performance thresholds. The clean run used
459% CPU on average and 958,028 KiB maximum resident memory; the warm run used
330% CPU and 958,040 KiB maximum resident memory. Neither run swapped.

The target occupied 10,488,172,544 bytes, 17,790 files, and 20,931 entries
after clean CI, including 1,792,479,232 bytes of Trybuild artifacts. The
unchanged rerun ended at 10,487,967,744 bytes with the same file and entry
counts. Normal unchanged reuse therefore showed no target growth beyond small
ephemeral cleanup variance.

### Common source and cache matrix

The table below contains single screening observations for invalidation and
ownership behavior. They are not presented as qualification medians. Full CI
was used where service ports were available; focused public composites were
used where a case concerned only binary tests or database inputs.

| Case | Surface | Result | Wall/task observation |
| --- | --- | --- | --- |
| no MFM cache | fresh root, full CI | 13/13 passed | 304.51s |
| exact unchanged | same root and path, full CI | 13/13 passed | 204.01s |
| documentation-only | README digest changed and restored, full CI | 13/13 passed | 189.67s; Nextest compile 16.06s |
| leaf crate | `mfm-authored-config` source changed and restored, full CI | 13/13 passed | 261.04s; Nextest compile 30.68s |
| shared crate | `mfm-ids` source changed and restored, full CI | 13/13 passed | 255.02s; Nextest compile 36.86s |
| manifest | authored-config package description changed; lock unchanged | 2/2 test leaves passed | 131.85s; Nextest compile 34.17s |
| verification profile | test codegen units changed to 255 and restored | 2/2 test leaves passed | 150.17s; Nextest compile 49.12s |
| non-Rust input | migration SQL comment changed and restored | 6/6 database leaves passed | 91.05s; online SQLx 6.08s |
| identical second path | same commit and state root from detached worktree | 13/13 passed | 230.17s; Nextest compile 31.35s |
| explicit Linux target configuration | focused `mfm-ids` check with explicit host triple | passed twice | 2.49s first, 0.06s warm |
| cacheless/bypass proxy | fresh isolated state root followed by normal reuse | passed | represented by the clean/warm pair |

The tracked mutation digests were:

| Input | Original | Disposable variant |
| --- | --- | --- |
| `README.md` | `29e3d87ae4bf3fb36ef639f4375fcec1f80b8dc5e748c4445b80df4410447f5c` | `176232b15219158ff28e68c93c91cec76c14733a1d2126cae9801cda14d8077a` |
| authored-config source | `e0ccfcd31b1f8ea91948f1805e67f647fd4b8d3081dfe94eee00a84367abe1e0` | `2d56d9ca7f19abb9de057f31649bd22ecead0b7edd086165c87e2858fd3d0494` |
| `mfm-ids` source | `c29ae93aa1b09a45d80a38eae5af521257534dba9386f066b0e1f4802bfc0510` | `933820e24980f6e575072c043658beb45ee8f38202f1bed9e1dfca2345e3a5a7` |
| authored-config manifest | `047dc4fdf9e34315c800e60d833a59385b8ae725307f6b5265fb5707636122a1` | `1c3bb7d886a19a8ddeb5d28bad98dbe1a9d3a707cf909f9e945cb99a559af17a` |
| migration `0002` | `0c5a4ca0ac25cc220c95e97e45e2886cb4feec4cb08963be44abe716a53315ea` | `cf72eda580584982a6a2f78389def51f84218e43fedba5b5c7dff4f9c9722022` |
| `nixfied.nix` profile | `5424200b21aacc28a881dff8e60142fc6bbc2012f7e92c95f4dc65a9a311c556` | `ef8f502496aa88d2e4c3eb2817307fd90ab988033147e07aee9800c762ccec72` |

`Cargo.lock` remained at
`e92a0a82bbaf96a522cc2eb3d1f669f7af6b0607cffc01d7682f96a6d8925e7c`
during the manifest probe. The profile variant changed the model hash to
`72cbd34cf1073968c64fe4b59779173951c63f04f77ab3ad2a37e1dce5e8db3c`
but intentionally retained the same cache digest. Cargo fingerprints safely
rebuilt the affected test units. Restoring the model and completing the later
recovery run proved normal default-profile reuse still passed.

The explicit target probe used the exact pinned compiler
`rustc 1.96.0 (ac68faa20 2026-05-25)`, host
`aarch64-unknown-linux-gnu`, and created a target-triple subtree rather than
reusing implicit-host artifacts. No second pinned toolchain was installed, so
cross-version compiler behavior was not timed. The cache key already includes
the declared toolchain and target identities; a real toolchain change remains
an identity-contract obligation rather than a measured alternate compiler in
R2-04.

There is no explicit cache-bypass control in the pinned runtime. A fresh
isolated state root is the only supported cacheless proxy, and it cannot prove
bypass behavior while preserving the same cache namespace. An explicit,
audited bypass remains a framework requirement.

### Worktree, slot, process, and port isolation

The identical second worktree reused the exact primary cache path because
scope is `slot`, not worktree. Cargo reused most artifacts, but source-path
sensitivity increased full CI by 26.16s relative to the unchanged run. The
target grew from 10,487,967,744 to 10,963,066,880 bytes; all 475,099,136 added
bytes were reflected in the Trybuild subtree, which grew to 2,267,697,152
bytes. Same-state/same-slot worktrees therefore share ownership and accumulate
path variants. MFM's current contract requiring separate state roots per
worktree is necessary but not runtime-enforced.

Two focused metadata-contract builds then ran concurrently from the primary
and detached worktrees in slots 1 and 2. Both passed in 30.28s/28.33s and used
distinct target paths of 1,235,656,704/1,235,374,080 bytes. A third focused
build in a separate worktree state root passed in 18.80s and used another
distinct 1,235,021,824-byte target. The digest remained identical in all
three locations; state-root and slot placement, not the digest string,
provided isolation.

Persistent Postgres instances also ran concurrently in slots 1 and 2 on
`127.0.0.1:28180` and `127.0.0.1:28280`, then stopped cleanly. In contrast,
two attempted full-CI probes failed before Cargo when an independent state
root used slot 0 on `127.0.0.1:28080`. Both failures reported
`PROC_ESCAPE`, PostgreSQL's address-in-use diagnostic, state-root, registry,
logs, and run summary. State roots do not namespace TCP ports; independent
processes must coordinate globally distinct slots.

### Same-slot concurrency, cancellation, and recovery

Two `.#test` processes started against the same state root, slot, source, and
target. Nixfied allowed both Nextest processes to run. Cargo's build lock
serialized planning/compilation where necessary, but test execution could
overlap; this is not a framework cache lease or exclusive-writer policy.

The first run was interrupted after its Nextest leaf had run 624/974 tests.
Its summary recorded `canceled: true`, null exit code, 41.566s task duration,
and eight SIGTERM-aborted tests; the log identified all aborted tests and the
350 tests not started. No child process survived. The concurrent second run
then passed 974/974 plus doctests in 103.61s. Its Nextest compile interval was
2.29s and execution was 83.106s. This proves cancellation propagation and
Cargo-target recovery for the observed case, but it also confirms that the
runtime does not itself prevent concurrent same-cache writers.

### Growth, inspection, retention, and garbage collection

The durable target retained every changed-source/profile variant:

| Point | Target bytes | Files | Entries | Trybuild bytes |
| --- | ---: | ---: | ---: | ---: |
| clean full CI | 10,488,172,544 | 17,790 | 20,931 | 1,792,479,232 |
| unchanged full CI | 10,487,967,744 | 17,790 | 20,931 | 1,792,299,008 |
| identical second worktree | 10,963,066,880 | 19,789 | 23,298 | 2,267,697,152 |
| after profile variant | 16,868,171,776 | 23,879 | 27,987 | 2,267,574,272 |
| after explicit-target probe | 16,924,336,128 | not recounted | not recounted | unchanged |

The accumulated leaf, shared-crate, manifest, and profile variants added
5,905,104,896 bytes after the cross-worktree sample while keeping the same
cache identity. The experiment did not capture a size boundary between every
one of those mutations, so that growth is not attributed to the profile alone.
No automatic age, size, profile-variant, or stale-unit pruning occurred. The
primary slot retained 13 run directories, but their logs and summaries
occupied only 2,813,952 bytes; compiler artifacts, not diagnostics, caused the
material growth.

Task summaries expose cache family, digest, mode, scope, and exact path. The
`ps` control exposes registered processes and reconciliation state. Neither
surface reports cache size, age, last use, owners, retained variants, or a
safe prune plan; those observations required direct filesystem traversal.

The only supported cleanup is slot-wide. On a disposable isolated root,
`nix run .#clean -- --slot 0` reclaimed a 1,236,123,648-byte slot in 0.85s.
It deleted the Cargo cache and the complete run directory together while
preserving the 98,304-byte process registry. A cleanup attempt with a live
persistent Postgres process correctly failed closed with `CLEANUP_REFUSED`
because active run leases existed; after `.#down`, the service stopped
cleanly. Active-process refusal is sound, but successful cleanup is too broad
for Cargo-cache retention because it also removes service state and run
diagnostics.

After all evidence had been extracted, supported broad cleanup removed the
three primary experimental slots in 2.24s/0.31s/0.39s. Available filesystem
space rose from 17,057,996,800 to 36,581,367,808 bytes, and no experiment
process or slot directory remained. This recovered the space safely but also
removed the runtime-owned run logs cited above, as the broad contract defines.

The Nix store grew from 26,642,591,744 to 26,756,800,512 bytes during model
and app realization, an increase of 114,208,768 bytes. The current CI and
model closures each remained approximately 1.5 GiB. Nix identified dead store
paths through its normal dry-run inspection, but Nix garbage collection cannot
see or reclaim mutable Cargo targets below `NIXFIED_STATE_DIR`.

### Ownership cost and R2-04 decision

Fresh isolation has simple ownership but high duplication: one complete
verification target costs approximately 10.49 GB and 304.51s clean, while
even focused metadata-only targets cost approximately 1.24 GB per slot or
state root. Persistence materially improves unchanged and many changed-source
workloads, but the operator must currently:

- allocate unique state roots per worktree and globally distinct slots for
  concurrent services;
- discover cache paths from task summaries and calculate size/age manually;
- monitor unbounded variant growth;
- choose between retaining all compiler variants or deleting the entire slot,
  including run and service evidence; and
- rely on Cargo's internal locking for same-target concurrency.

The persistent local Cargo target therefore performance-qualifies as the
control candidate: both R2-01 and R2-04 exceed the required clean-to-warm
threshold. It does not operationally qualify as the final durable architecture
because safe ownership, inspection, bounded retention, cache-family garbage
collection, and explicit bypass are missing. The target-only control remains
in the candidate comparison, but final selection requires framework support
rather than an MFM shell cleaner.

At R2-04 this report carried cache-family inspection, accounting, cleanup,
retention, bypass, worktree ownership, writer semantics, and actionable port
diagnostics forward as proposed Nixfied requirements. The final responsibility
correction rejects the compiler-cache items and assigns them to Cargo/MFM; only
the host-global endpoint diagnostic and acquisition defect remained an
upstream runtime responsibility.

## R2-05 follow-up: bounded `sccache` experiment

R2-05 tested `sccache` only in two disposable detached worktrees at source
`7eef087c9c549070f117b0320fb50ee8f6814186`. No wrapper, cache path, task, or
dependency was added to the repository. The isolated Nixfied variant added the
root flake's pinned `sccache` package, set a 5 GiB local cache and
`RUSTC_WRAPPER=sccache`, retained the nonincremental compact verification
profile, and overrode the online SQLx task with `RUSTC_WRAPPER=""`. Its
`nixfied.nix` SHA-256 was
`ac18f0cbd98d1704e94fcc451af86cd44578abe0eb619d7db74a96077c83387e`;
model admission passed with model hash
`3cad9a5ef677170b5422e1a272afa209688ac409648d12dec129f496b4009696`
and the unchanged runtime ABI.

The candidate used the root flake's pinned `sccache 0.15.0` at
`/nix/store/r1l6d3lw9qairswn87dizzc4ar7jykbs-sccache-0.15.0`, not the
newer package visible through the host's flake registry. Rust, Cargo, Nextest,
SQLx, Nix, Nixpkgs, Nixfied, target, host, and workload identities remained the
R2-01 reference identities. The cache lived outside the Nixfied state root at
`/tmp/mfm-r2-05-cache`; all timed full gates used slot 5 and ran serially.
Remote backends, exported archives, hosted persistence, and macOS performance
were outside the phase.

### Screening timings and cache statistics

The RFC requires at least two cacheless samples and three warm samples before
qualification. Both empty-cache runs deleted the Cargo target and the isolated
object cache. The retained-cache fresh-target runs deleted only the Cargo
target. Every full run passed all 13 CI leaves.

| State | Run | Full CI | Outer wall | Rust hits | Rust misses | Rust hit rate |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| empty cache A | `run-3927599-1784311312746378410` | 370.02s | 371.07s | 42 | 2,150 | 1.9% |
| empty cache B | `run-4028587-1784311700371396178` | 344.28s | 344.40s | 42 | 2,150 | 1.9% |
| exact same target | `run-4115596-1784312057775096187` | 170.03s | 170.29s | 281 | 30 | 90.4% |
| fresh target, same path | `run-4142338-1784312243024384030` | 217.62s | 217.72s | 2,192 | 0 | 100.0% |
| fresh target, second worktree | `run-21759-1784312512252792565` | 265.83s | 265.93s | 1,779 | 413 | 81.2% |

The empty-cache median was 357.74s. It regressed 53.02s/17.4% against the
R2-04 304.72s outer clean control, and each individual empty-cache sample was
more than 10% slower than that control. The warm median was 217.72s. A
same-path fresh target backed by the cache saved 87.00s/28.5% against the clean
control, but moving identical source to the second worktree reduced that
saving to 38.79s/12.7%. The exact same-target observation saved 33.96s/16.6%
against R2-04's 204.25s target-only warm control, but a single observation is
not a qualification median.

The two empty runs each issued 3,399 compiler requests. Of those, 555 were
not cacheable: 358 `crate-type`, 164 multiple-input, 23 stdin-like `-`, four
missing-input, three explicit-output, two missing-output-directory, and one
argument-parse request. This captures final binaries, proc-macro/final crate
types, and link-like work that the object cache cannot serve. Rustdoc remained
outside the wrapper, and doctest execution remained residual work. Trybuild
continued to run inside Nextest; its final test executables were likewise not
cacheable.

The same-path fresh-target run served 2,816 Rust/C/assembler hits with 6.69s
of aggregate cache-read-hit time, about 2.4ms per hit, while still spending
217.62s end to end. Cache hits therefore removed compiler work but did not
remove Cargo planning, final linking, rustdoc, test execution, database
startup, or parity execution.

### Coverage, edits, paths, and bypass

Every full-CI sample passed 974/974 Nextest tests, 53 doctest binaries with 42
tests, nine Trybuild harnesses with 80 UI sources and 71 checked stderr
baselines, offline and online SQLx, and the keystore, CLI, REST, and Postgres
parity leaves. Regenerating the exact Nextest list from the candidate target
produced 974 identifiers across 99 binaries. Sorting
`<binary-id>::<test-name>` in byte order with a trailing newline produced the
frozen SHA-256
`e75d6ea039b5507c6f9b89bef89656e31073c02f8f17f74f680fc2bbf0d67f08`.

The screening edit and recovery observations were:

| Case | Disposable change | Result | Rust hits/misses | Outer wall |
| --- | --- | --- | ---: | ---: |
| leaf crate | authored-config maximum `256 * 1024` to `256 * 1024 + 1` | 974/974 plus doctests passed | 250/66 | 85.13s |
| shared crate | add `#[inline]` to `mfm-ids::IdentityError::message` | 974/974 plus doctests passed | 59/363 | 128.70s |
| explicit bypass | identical model with `RUSTC_WRAPPER=""` | 974/974 plus doctests passed; zero wrapper requests | 0/0 | 123.70s |
| reuse after bypass | restore the wrapper and original sources | 974/974 plus doctests passed | 290/11 | 76.30s |

The original/variant SHA-256 pairs were
`e0ccfcd31b1f8ea91948f1805e67f647fd4b8d3081dfe94eee00a84367abe1e0`/
`4ce8a3a1f5c7d5c5cc47ca0777dc870095e1564f1c4e0e38146db0adf8d79cfb`
for the leaf source and
`c29ae93aa1b09a45d80a38eae5af521257534dba9386f066b0e1f4802bfc0510`/
`e75f7baf4bb76c46f66d6af17939d5471dc3ad9e9a39234781e6ce568a5d1627`
for `mfm-ids`. The bypass model variant had SHA-256
`ca02814b2c24f144416b9b783858b574269e50f4d44569d36c4967704ba3f5e8`.
All mutations were restored.

The second worktree used the same candidate-file digest and the same Cargo
target path after that target was cleaned. Its 413 Rust misses show that
source-path normalization is incomplete for this workload. Two simultaneous
`mfm-ids` builds from the two worktrees, with separate Cargo targets and one
64 MiB cache/server, both exited successfully. The server reported 16 misses,
16 writes, no write errors, and no corruption. A local daemon serializes cache
access adequately for that observed case, but the repository still has no
declared cross-user trust or ownership policy for such a mutable cache.

The compiled model records `RUSTC_WRAPPER=""` and `SQLX_OFFLINE=false` on
`postgres-sqlx-check`, while offline SQLx and the remaining Cargo tasks use the
wrapper. The online disposable-schema mutation and recovery check passed in
all five full-CI samples. Thus online preparation did not trust object-cache
reuse.

### Filesystem inputs and false-hit probes

Two probes changed only files read during compilation while leaving their
Rust callers byte-identical:

- `runtime-config`'s `include_str!` input changed from SHA-256
  `339cb41a8d0d8ac680d8a728829cbf00aa9a1fc95150f31234a9ba98075bdabe`
  to `961087d43358feecbb4a41ab6fd019c3a9a7269171b67f9d84b0b38f8bea74ba`.
  The affected final test crate was classified non-cacheable, all 19 focused
  tests passed, and its binary contained the new marker. No stale object was
  eligible for a hit.
- migration `0002` changed from SHA-256
  `0c5a4ca0ac25cc220c95e97e45e2886cb4feec4cb08963be44abe716a53315ea`
  to `63e6bd31d534639577b2ba29a6eec01dbad716706f8657ab7e85c53111e2adab`.
  After deleting and recreating the same Cargo target against the retained
  cache, 170 Rust requests hit and exactly one missed. The rebuilt
  `mfm-stream-store-postgres` rlib contained the new migration marker. This
  observed SQLx filesystem-reading macro invalidated correctly.

Both probes used the pinned wrapper and the compact nonincremental profile;
both changes were restored. These observations reject a false hit for the
tested include and migration paths, but they are not a general proof for every
third-party filesystem-reading proc macro.

### Bounds, eviction, corruption, and cleanup

The 5 GiB screening cache occupied 1,027,230,637 bytes after either empty
full-CI run, 1,294,661,446 bytes after the second-worktree run, and
1,539,002,021 bytes after the edit/bypass recovery cycle. It stayed within its
configured bound without reaching eviction.

A separate 32 MiB stress cache reached 32,722,469 bytes after a focused
Postgres-store seed and 32,831,690 bytes after an immediate clean-target
rebuild, both below the 33,554,432-byte logical bound before small directory
overhead. The rebuild had zero hits and repeated all 171 Rust misses, proving
that eviction enforced the bound but could eliminate all useful reuse when
undersized. Version 0.15.0 exposed hit/miss and current/max size statistics but
no eviction counter, so the repeated misses plus bounded size are the observed
eviction evidence.

For corruption recovery, a separate 64 MiB cache seeded eight `mfm-ids` Rust
objects. With the daemon stopped, every object was deliberately truncated;
the same-path Cargo target was then recreated. The next build succeeded,
reported eight Rust cache errors and eight misses, recompiled all eight
objects, and rewrote a healthy 6,665,254-byte cache. Corrupt entries were not
returned as successes.

The daemon has an explicit stop operation, while cache removal is ordinary
filesystem deletion outside Nixfied's cache-family model. R2-05 used only
exact, experiment-owned directories and stopped each daemon before deletion.
This is technically cleanable but does not supply the inspection, lease,
trust, retention, or cache-only lifecycle contract required from Nixfied in
R2-04.

### R2-05 decision

`sccache` is rejected as a standalone environment-specific candidate at
screening. It preserved coverage, handled the tested invalidations and
corruption safely, supplied an explicit bypass, and materially accelerated a
same-path fresh target. However, both empty-cache samples exceeded the
target-only clean control by more than the RFC's allowed 10% regression, the
empty-cache median regressed 17.4%, and identical-source reuse fell below the
15% materiality threshold after changing worktree path. Final links, test
binaries, proc-macro/final crate types, rustdoc, and residual execution remain
outside its useful cache surface.

Per the screening contract, R2-05 did not expand to the three-clean/five-warm
qualification matrix or spend additional runs on documentation-only,
manifest/lockfile, verification-profile, alternate-toolchain, explicit-target,
and every build-script/trybuild-baseline variant. Those are qualification-only
cases after the screening veto, not silently verified surfaces. No remote,
archive, hosted, or macOS claim is made. The failed candidate is not combined
with Crane or another Nix-native compilation mechanism, and no pilot code is
retained.

## R2-06 follow-up: consumed Crane verification artifact

R2-06 tested Crane only in a disposable detached worktree at source
`75e2210d`. The pilot pinned Crane 0.23.3 at revision
`db220b8709cea4bd43c9adcd4192f212fb6768d1` with NAR hash
`sha256-K0i9GoNk2To1RQkW348EY4c7RYf3mD74hWlGSc/9sk0=`. It used the exact
R2-01 Rust, Cargo, Nextest, Nixpkgs, Nixfied, feature, compact profile, and
Linux host identities. No generated or repository-owned stand-in Rust source
was used.

The pilot constructed a `buildDepsOnly` dependency output and passed it as
`cargoArtifacts` to a final `mkCargoDerivation`. The final derivation built a
Nextest archive from the immutable, content-addressed workspace source while
using only the sandbox target as mutable build state. Its derivation inputs
contained the exact dependency derivation, its environment named the
dependency output, and its log reported decompression of that output before
compilation. The final output contains the compressed archive and a symlink to
the immutable source; it contains no copied mutable Cargo target.

An initial archive built from the sandbox checkout preserved
`CARGO_MANIFEST_DIR=/build/source` in binaries and could not execute after the
sandbox disappeared. A supported `--remap-path-prefix` probe did not change
the compile-time `env!` value. Building from the immutable source store path
fixed the underlying relocation boundary. The corrected archive contained no
`/build` reference, executed successfully, and retained actionable immutable
source paths.

The opt-in Nixfied model consumed a verification bundle in the
`crane-verification` task closure and ran pinned Nextest directly against the
archive with its immutable workspace remap. Model admission passed. An
opt-in `crane-ci` composite replaced only the public Nextest leaf; check,
doctests, keystore, offline and online SQLx, and the CLI, REST, metadata, and
Postgres parity leaves remained the public definitions. The checked-in model,
public apps, and public gates were not changed. Evaluating the archive took
0.94s; evaluating an unrelated public quick task took 1.04s and did not
realize or reference the Crane closure.

### Artifact construction, consumption, and size

The first dependency realization built 829 derivations in 197.94s wall with
2,799,180 KiB maximum RSS. Most were locked registry-source and package
realizations. The dependency target was 1.85 GiB before compression; its
single `target.tar.zst` was 441,757,979 bytes, its NAR was 441,758,272 bytes,
and its closure was 972,405,248 bytes across 458 paths. An exact local-store
hit took 0.15s.

The corrected final archive rebuilt the workspace in 52.41s wall. It archived
100 binaries, including 99 test binaries and one CLI binary, plus one build
script output directory, one linked path, and one standard library. The
compressed archive is 757,966,616 bytes; the output is 757,966,666 bytes, its
NAR is 757,967,488 bytes, and its closure is 766,343,584 bytes across two
paths. The 242-byte symlink bundle has a 1,440-byte NAR and an 855,667,112-byte
execution closure across 12 paths.

The immutable execution extracted 100 binaries, discovered exactly 974 tests
across the expected 99 test binaries, and passed all 974. Sorting
`<binary-id>::<test-name>` in byte order with a trailing newline produced the
frozen SHA-256
`e75d6ea039b5507c6f9b89bef89656e31073c02f8f17f74f680fc2bbf0d67f08`.
All nine Trybuild harnesses passed. Trybuild necessarily remained a nested
Cargo consumer: one execution produced 1,762,101,961 bytes of mutable residual
target data, entirely below `tests/trybuild`. The archive therefore removes
the primary Nextest compilation lane but not nested UI-test compilation.
Doctests, online SQLx, and live Postgres parity likewise remained explicit
Cargo/service leaves in the full composite rather than being claimed as
archive coverage.

### Screening timings

The first corrected immutable execution took 106.63s outer wall. Two exact
artifact reruns took 81.81s and 81.48s outer wall. The full equivalent
composite passed all 13 leaves in every observation:

| State | Full composite task | Outer wall | Comparison with R2-04 |
| --- | ---: | ---: | ---: |
| retained artifact, clean residual target | 274.94s | 271.36s | 33.36s/10.9% faster than 304.72s clean |
| warm A | 199.60s | 195.74s | 8.51s/4.2% faster than 204.25s warm |
| warm B | 198.97s | 195.20s | 9.05s/4.4% faster |
| warm C | 190.75s | 188.81s | 15.44s/7.6% faster |

The warm outer median was 195.20s, only 9.05s/4.4% faster than the R2-04
204.25s warm control. The retained-artifact clean-residual observation saved
10.9%. Both miss the RFC's simultaneous 15% and 30-second materiality gate.
Moreover, a truly absent-artifact first use must realize the 197.94s
dependency output and build the approximately 52-second archive before the
approximately 107-second execution; that cold chain is roughly 358 seconds
before small model/launcher overhead and is slower than the target-only clean
control.

Per the screening contract, these results stop qualification. R2-06 did not
spend additional runs on a three-clean/five-warm distribution. The matrix
below completes invalidation and correctness screening without presenting
those probes as timing qualification.

### Source, cache, relocation, and concurrency matrix

The base dependency and archive derivations were respectively
`rjyb99dga0sfars58yf85d9mv3n5ajjp` and
`l9ccmwzfinq0qqgw9ycpssy4m3z0wlc7`:

| Case | Dependency artifact | Final archive | Execution observation |
| --- | --- | --- | --- |
| absent artifact cache | built | built | 974/974 passed |
| exact unchanged rerun | exact store hit | exact store hit | 974/974 passed |
| tracked README edit | same derivation | same derivation | non-source input excluded |
| leaf authored-config edit | same derivation | changed and rebuilt in 54.86s | 974/974 passed in 103.18s task wall |
| shared `mfm-ids` edit | same derivation | changed and rebuilt in 52.58s | 974/974 passed in 81.21s task wall |
| manifest description edit | same derivation | changed | derivation-boundary probe after screening stop |
| `Cargo.lock` edit | changed | changed | derivation-boundary probe after screening stop |
| verification-profile edit | changed | changed | derivation-boundary probe after screening stop |
| migration SQL edit | same derivation | changed and rebuilt in 47.35s | 974/974 passed in 103.11s task wall |
| identical second worktree | exact same derivations | exact same store output | 974/974 passed in 104.02s task wall |
| Linux target configuration edit | changed | changed | derivation-boundary probe after screening stop |
| outputs absent, then normal reuse | recovered six derivations | rebuilt, then exact hit | 974/974 passed in 175.41s outer wall |

The leaf mutation changed authored-config's maximum from `256 * 1024` to
`256 * 1024 + 1`; its variant SHA-256 was
`4ce8a3a1f5c7d5c5cc47ca0777dc870095e1564f1c4e0e38146db0adf8d79cfb`.
The shared mutation added `#[inline]` to
`mfm-ids::IdentityError::message`; its variant SHA-256 was
`e75f7baf4bb76c46f66d6af17939d5471dc3ad9e9a39234781e6ce568a5d1627`.
The migration probe changed SQL SHA-256 from
`0c5a4ca0ac25cc220c95e97e45e2886cb4feec4cb08963be44abe716a53315ea`
to `4d17822130e9d00f038baa30938d352db8a29a987115b7b56579f1238fb12e17`.
All source, manifest, lockfile, profile, and target mutations were restored.

Two simultaneous executions from separate worktrees and state roots initially
failed closed while extracting the archive because only 5.1 GiB remained on
the filesystem. After supported cleanup of prior experiment slots restored
11 GiB free, the same bounded two-slot probe passed 974/974 in both slots in
148.43s and 148.44s outer wall. There was no shared mutable compiler state or
corruption, but concurrency needs roughly twice the archive extraction and
Trybuild residual space; the first failure is an important capacity cost, not
a correctness success hidden by a retry.

### Retention, deletion, recovery, and decision

The dependency, archive, bundle, model, and launcher outputs are ordinary Nix
store objects. Exact output deletion after removing their referrers reclaimed
1.1 GiB. Locked registry/vendor store paths remained reusable normal Nix store
objects rather than a private MFM cache. With the pilot outputs absent, one
opt-in command rebuilt the six missing pilot derivations and passed all 974
tests in 175.41s outer wall. A subsequent exact invocation reused the restored
outputs. Nix store validity, referrer, path-info, closure-size, NAR-size, and
garbage-collection dry-run inspection all worked; mutable Trybuild residuals
remained owned by the Nixfied slot and required its supported slot cleanup.

Coarse Crane verification is rejected at screening. It satisfied the hard
artifact contract: the dependency output was genuinely consumed, the final
artifact was immutable and relocatable, exact coverage matched, non-Rust and
Rust invalidation behaved correctly, all nine nested-Cargo harnesses passed,
the Nixfied task consumed the closure directly, and public gates remained
authoritative. It nevertheless saved only 4.4%/9.05s at the warm full-gate
median and 10.9%/33.36s with a retained artifact and clean residual target,
while adding roughly 442 MB of dependency output, 758 MB of final archive,
large first-realization cost, extraction I/O, and approximately 1.76 GB of
mutable Trybuild state per execution slot.

No Crane input, package, task, model, generated source, cache, or workflow is
retained in the main worktree. R2-07 should compare this coarse baseline with
real granular `crate2nix` derivations; it must not treat Crane's correctness as
evidence that a per-crate graph will qualify.

## R2-07 follow-up: granular `crate2nix` artifacts

R2-07 tested crate2nix only in a disposable detached worktree at source
`b2b5778a`. The pilot pinned crate2nix 0.15.0 at revision
`7c33e664668faecf7655fa53861d7a80c9e464a2` with NAR hash
`sha256-SUuruvw1/moNzCZosHaa60QMTL+L9huWdsCBN6XZIic=`. It retained the R2-01
Nixpkgs, Nixfied, Rust 1.96.0, Cargo, and aarch64-linux identities. The only
build-policy adapter selected the MFM Rust toolchain, development mode, 256
codegen units, and debuginfo level 1. No native-dependency or platform crate
override was required.

Manual generation produced a 20,579-line, 697,704-byte `Cargo.nix` with
SHA-256 `1b78d5052fe1b1634e07303eecbad793242c5dd77eda8df7daafc39f47abeaf0`.
Regenerating at the same repository path was byte-identical. The first
standalone generator realization took approximately 2 minutes 41 seconds;
warm generation took 0.37-0.59s with about 105 MiB maximum RSS. Generation to
a different output path was not byte-identical because the generated header
and relative source paths encode that path, so the deterministic workflow must
regenerate the committed path.

The generated graph contained exactly the same 504 unique package name/version
pairs as `Cargo.lock`: neither side had an unmatched pair. Cargo metadata
reported 53 workspace members, 504 total packages, 2,043 targets, 71 build
scripts, 33 proc macros, and five packages with native `links`; the generated
graph represented all 33 proc macros and all five `links` packages. The MFM
workspace itself contributed 100 targets and one build script. Target
predicates, host/build dependency separation, feature edges, build scripts,
proc macros, and native link names were present in the generated graph. This
establishes structural lockfile fidelity, but not Cargo-equivalent verification
behavior.

Committed generation has no intrinsic stale-file guard. Changing one locked
checksum left `Cargo.nix` and the evaluated `mfm-ids` derivation unchanged;
regeneration failed through Cargo metadata with the corrupt-lock diagnostic.
A repository-owned regeneration comparison would therefore be mandatory on
every dependency or feature change. In contrast, crate2nix's IFD helper kept
the generated graph coupled to the source but required 998 first-realization
derivations and 5:44.83 wall, 3,579,124 KiB maximum RSS, and 6,146,184 KiB of
filesystem output on this host. Importing the realized generated graph and
evaluating one leaf then took 0.97s. The IFD path also requires
`allow-import-from-derivation`; the manual path avoids that evaluator policy at
the cost of committed generated-file maintenance.

### Build graph, invalidation, and storage

Building all 53 workspace member roots as real crate2nix derivations succeeded
for the non-test graph. It required 1,091 derivations and 15:34.87 wall, with
1,540,184 KiB maximum RSS and 34,298,120 KiB of filesystem output. The aggregate
output was only a link farm: its NAR was 800 bytes, while its 18-path runtime
closure was 732,961,096 bytes. Evaluating that aggregate derivation took 3.51s
and 486,552 KiB maximum RSS; a single `mfm-ids` derivation took 0.42s and
135,364 KiB.

The graph did provide genuine granular reuse for a simple leaf. Two immutable
`mfm-ids` test executables occupied 15,022,456 bytes, had a 15,023,176-byte NAR
and a 74,280,064-byte seven-path closure, and all eight tests passed when run
directly from the store. Exact deletion reclaimed 14.3 MiB; rebuilding the one
missing derivation restored the identical output in 1.19s, after which store
verification passed.

Granularity was much weaker across all workspace roots. A tracked README edit
changed neither the leaf nor aggregate derivation. A one-line `mfm-ids` source
edit changed 263 of 1,724 aggregate derivation-closure paths, including 261 MFM
crate variants. Each workspace member root resolves its own feature graph, so
widely shared crates are rebuilt in many feature combinations rather than once
for the workspace. Lockfile changes do not invalidate a committed generated
graph until regeneration occurs. Crate-local migrations and `.sqlx` metadata
are included by the broad per-crate source filter, as are crate-local Trybuild
sources and stderr files. Repository-level examples and sources belonging to
sibling workspace crates are outside each crate source and are neither build
inputs nor invalidation inputs for that crate.

### Verification artifact veto

The pilot failed the RFC's hard coverage screen before performance
qualification. A real `mfm-portfolio-config` test target failed to compile in
2.40s because crate2nix supplied only `crates/portfolio-config`; the test's
`include_str!("../../../examples/configs/portfolio-dual-mainnet.toml")` could
not read the repository-level example. The same unsupported source shape is
used by runtime-config, app tests, and integration tests, while app and
integration tests also read sibling workspace sources. Fixing this would
require widening or fabricating crate sources and maintaining semantic source
overrides, which R2-07 explicitly forbids and crate2nix documents as a known
workspace-source restriction.

The public crate2nix `runTests` interface is also not a reusable verification
artifact. It copies test executables into a derivation-private mutable target,
runs them while the sandbox source exists, records their combined output, and
returns the linked normal crate. The compiled harnesses are not exposed for
authoritative repeated Nixfied execution. A private internal graph function
could expose binaries, but that API is explicitly unstable and did not repair
runtime semantics: the 40,063,312-byte `mfm-program` test output had a
375,807,120-byte 16-path closure, yet its Trybuild harness failed immediately
outside the build sandbox. Tracing showed that it derived an empty project name
from the unavailable compile-time manifest location and invoked Cargo with
`--bin -tests`. Trybuild remains a nested Cargo consumer and needs the real
workspace source, lockfile, toolchain, registry inputs, and writable target at
execution time.

Consequently the pilot could not produce the frozen 99-test-binary, 974-test
identifier inventory, exercise all nine Trybuild harnesses, or compare final
binary identities with the authoritative Cargo/Nextest path. Doctests, online
SQLx checks, and live Postgres parity likewise remained separate Cargo/service
work. No Nixfied task was added: wiring a convenient eight-test subset or a
cached build-time success would violate the requirement that a verification
task directly consume a complete immutable artifact. There was therefore no
full-gate timing distribution, concurrency qualification, or platform claim
beyond the reference aarch64-linux host.

Granular crate2nix artifacts are rejected at screening. The experiment proved
that crate2nix can generate an exact package/version graph, build MFM's
non-test crates, cache simple leaf test binaries, recover ordinary Nix store
outputs, and preserve coarse source locality. It cannot represent MFM's actual
cross-workspace verification sources or expose a supported complete artifact
for authoritative repeated execution. Its full-root graph also cost over 15
minutes locally, multiplied a shared edit into 261 MFM derivation variants,
and added a large generated-file/IFD and private-API maintenance surface.

No crate2nix input, lock entry, `Cargo.nix`, package, override, task, model,
generated source, cache root, or workflow is retained in the main worktree.
Public gates and the authoritative Cargo path were never changed. R2-08 should
test `cargo2nix` independently against the same hard source and consumption
contract; it must not inherit either this rejection or any favorable
crate2nix result.

## R2-08 follow-up: granular `cargo2nix` artifacts

R2-08 tested cargo2nix only in a disposable detached worktree at source
`96f58f6e`. The pilot pinned cargo2nix 0.12.0 at revision
`a709c74619e1a2b68ed12bb398e12fbe29d69657` with NAR hash
`sha256-l06DIwnB4JHwP1isUUXk85F+AHQkUSUyAWnAmRxXICg=`. It retained the R2-01
Nixpkgs, Nixfied, Rust 1.96.0, Cargo, and aarch64-linux identities. The
upstream combined overlay could not do that: its older Rust overlay hid Rust
1.96.0 and failed evaluation with a missing-version error. The pilot therefore
used the separately supported cargo2nix builder overlay after MFM's Rust
overlay and passed MFM's exact toolchain to `makePackageSet`.

The cargo2nix executable is itself a granular package set built with upstream's
Rust 1.83.0 and Cargo library graph. An early bootstrap was stopped after its
large independent graph became clear. Completing the bootstrap from that
partial cache still required 586 derivations, 12:22.42 wall, 2,673,644 KiB
maximum RSS, and 15,597,464 KiB of filesystem output. Its final executable had
a 1,440,919,592-byte unique closure NAR total and no GC root. This is a large
generator bootstrap and toolchain maintenance surface before any MFM crate is
built.

Locked generation then completed in 1.41s with 193,100 KiB maximum RSS and
produced a 7,874-line, 488,422-byte `Cargo.nix`. Its SHA-256 was
`14693c667485abbe59cfa628bc3b68ad023e1cc06dfd0d76ec91681324f0f55c`.
Generation to stdout was byte-identical. The graph embedded cargo2nix 0.12.0
and the exact `Cargo.lock` SHA-256
`e92a0a82bbaf96a522cc2eb3d1f669f7af6b0607cffc01d7682f96a6d8925e7c`.
Changing only the lockfile changed the current hash to
`6d08c3d789f9d36b6deab2021ef501ca6ab7f38f6370894ae9b035c15e243443`
and made package-set evaluation fail with both hashes in the diagnostic. The
committed graph therefore has a useful intrinsic drift guard, although it must
still be regenerated by the pinned older generator when Cargo metadata or
features change.

The generated graph contained 504 crate nodes and 53 workspace members, equal
to Cargo metadata's 504 packages and 53 members. It accepted the current lock
format and represented host/build dependencies, build scripts, proc macros,
target conditions, and native-link packages well enough to begin a real MFM
build. That structural equality did not imply Cargo-equivalent dependency or
feature selection, as the broad build demonstrated.

### Artifact behavior, invalidation, and storage

A development-mode `mfm-ids` library build required 14 derivations and
13.44s, with 453,132 KiB maximum RSS and 1,109,480 KiB of filesystem output.
Its test-mode increment required one derivation and 1.42s. Cargo2nix exposed
two immutable test executables; all eight tests passed twice by direct store
execution. A repeated Nix realization of the library and test outputs took
0.21s. The library closure had a 74,896,080-byte unique NAR total and the test
binary closure 61,786,272 bytes. This proves supported leaf-level immutable
artifact production and ordinary Nix reuse.

Local workspace granularity was not preserved. Every generated local crate
uses `fetchCrateLocal workspaceSrc`, and the pilot supplied the whole
repository so MFM's repository-level examples, sibling crate sources,
migrations, `.sqlx` files, and Trybuild sources were available. The inspected
store source included both migrations, both `.sqlx` files, 48 program UI
files, repository examples, and the REST API source. This avoids crate2nix's
missing-source shape, but makes the entire tracked repository the source of
every one of the 53 local crates.

The measured `mfm-ids` derivation changed from
`jfz9i41lh2jyn3higa62nssr3zfscn6x` to
`kgbz2i34c3m25znad3qlsi5q2rrp49vh` after only a tracked README edit. Its test
derivation and the all-workspace aggregate changed too. An untracked file did
not change the derivation. Editing only the pilot's Nix override consequently
scheduled 149 derivations on the retry, including local crates unrelated to
that override. Cargo2nix provides granular third-party crates here, but all
tracked source, documentation, generated graph, lock, and pilot-wiring changes
invalidate every reachable MFM crate rather than the owning crate alone.

### Dependency graph and verification vetoes

The all-workspace test-binary aggregate evaluated to 3,029 derivation-closure
paths and initially scheduled 697 builds. After 8:40.93, 926,356 KiB maximum
RSS, and 28,281,168 KiB of filesystem output, `sha3-asm` failed because its
build script invokes Perl and cargo2nix's stock overrides did not propagate
Perl. One explicit package override adding build-platform Perl fixed that
native build path. The source-wide invalidation retry scheduled 149
derivations and ran for another 1:06.62, with 514,604 KiB maximum RSS and
7,707,864 KiB of filesystem output.

The retry exposed a correctness veto rather than another missing native tool.
MFM enables SQLx 0.9.0 with default features disabled and only Tokio, no TLS,
Postgres, macros, migrations, and JSON as applicable. Cargo reports no reverse
dependency at all for `sqlx-mysql` or `sqlx-sqlite`. Generated `Cargo.nix`
listed only the requested Postgres feature set on `sqlx`, but nevertheless
wired both optional database crates as unconditional dependencies. It then
built `sqlx-sqlite` with a stock system-SQLite override and failed because the
selected `libsqlite3-sys` surface lacked `sqlite3_prepare_v3` and
`SQLITE_PREPARE_PERSISTENT`. Removing optional edges or repairing their feature
sets with MFM overrides would be a hand-maintained Cargo unit-graph shim, which
R2-08 explicitly forbids. Package/version count equality therefore masks a
material feature and dependency graph mismatch.

The partial build produced 28 test binaries for 18 current-source workspace
crates before 35 workspace test crates were blocked. Direct execution still
showed useful ordinary behavior: 34 `mfm-program` unit tests and seven
`mfm-program-derive` descriptor tests passed from immutable outputs. Trybuild
did not. Both program UI harnesses derived an empty generated package identity
and invoked ambient Cargo as `cargo check --bin -tests`; Cargo rejected
`-tests` as an option. Setting `CARGO_MANIFEST_DIR` to the immutable whole
workspace source exactly as cargo2nix's public `runTests` helper does produced
the same failure. The test-binary closure itself contains no Cargo or Rust
toolchain, so the public runner also lacks the nested Cargo runtime that
Trybuild requires.

These independent failures prevented the pilot from producing or executing
the frozen 99-test-binary, 974-test identifier inventory and all nine Trybuild
harnesses. Doctests are not part of cargo2nix's `cargo build --tests` artifact;
online SQLx checks and live Postgres parity remain Cargo/service work. No
Nixfied task was added because neither a convenient eight-test leaf nor a
partial 28-binary aggregate is an authoritative verification artifact. There
was consequently no exact test parity, complete repeated Nixfied execution,
full-gate timing distribution, concurrency qualification, or platform claim
beyond the reference aarch64-linux host.

Granular cargo2nix artifacts are rejected at screening. The experiment proved
deterministic generation, intrinsic lock drift detection, exact top-level
package/member counts, whole-workspace source availability, direct immutable
leaf tests, and warm Nix reuse. It failed Cargo feature/dependency fidelity,
native build completeness without an MFM override, Trybuild execution, exact
coverage, and useful locality for tracked MFM changes. The large independent
generator bootstrap, 488 KiB generated graph, overlay ordering requirement,
custom native override, and older generator/toolchain graph add operational
cost without yielding an authoritative artifact.

No cargo2nix input, lock entry, `Cargo.nix`, package, override, task, model,
generated source, cache root, or workflow is retained in the main worktree.
The disposable worktree was removed and its store outputs have no roots, so
they are eligible for normal Nix garbage collection. Before the experiment,
the supported `nix run .#clean -- --slot 7` path removed an obsolete 9.9 GiB
measurement slot under cleanup id
`cleanup-975935-1784322991901110868`; public slot 0 and its state were
preserved. Public gates and the authoritative Cargo path were never changed.
R2-09 may compare only candidates that passed screening and therefore has no
`crate2nix` or `cargo2nix` qualification run to perform.

## R2-09 follow-up: qualifying candidate comparison

R2-09 was measured on 2026-07-17 from committed R2-08 revision
`67f4d476d7ca76b04f32eccdbd7ddf17d6cf325d`. It retained the R2-01
reference aarch64-linux guest, Rust/Cargo 1.96.0, compact verification
profile, Nixfied model hash
`d25c647af500ec8c060e173f05e199564cf3d3921663c82f04c3477a4c49ce84`,
runtime ABI `nixfied-runtime-abi:1-5ff3aa14f2bf`, and Cargo-target digest
`dd27515db6a0fc48aabe31c5e300af7325e3af24a98f0560364fa9c516971e03`.
No candidate implementation or default architecture was added.

Only the scoped Cargo target passed its standalone screening gate. R2-09
therefore ran the required three-clean/five-warm qualification distribution
only for that candidate. `sccache`, Crane, crate2nix, cargo2nix, and execution
topology retain their independently measured screening evidence; rerunning a
qualification distribution for any of them would violate the RFC's screening
stop. No combined candidate was authorized, and neither component of a
possible combination independently qualified.

### Scoped Cargo-target qualification

Three independent state roots supplied the clean observations. Five warm
observations reused the same source path and each root's slot-scoped target.
All eight runs passed the authoritative 13 leaves. No observation was
discarded, including the slower first clean run.

| State | Run | Full CI | Nextest | Compile/link | Execution | Doctest | Verification tail |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| clean A | `run-1396470-1784325967789774341` | 422.086s | 185.742s | 50.66s | 134.594s | 25.496s | 131.260s |
| clean B | `run-1509760-1784326661941504559` | 359.204s | 167.630s | 59.43s | 107.625s | 16.246s | 112.606s |
| clean C | `run-1567583-1784327274808964637` | 320.486s | 150.318s | 50.67s | 99.231s | 15.334s | 107.012s |
| warm A | `run-1479997-1784326411189162122` | 244.187s | 118.620s | 32.94s | 85.204s | 15.884s | 102.202s |
| warm B | `run-1549753-1784327027489871992` | 230.327s | 117.165s | 29.19s | 87.537s | 15.068s | 87.941s |
| warm C1 | `run-1605668-1784327601521425968` | 197.644s | 93.515s | 21.24s | 71.882s | 14.253s | 83.105s |
| warm C2 | `run-1619084-1784327808300874757` | 194.250s | 91.253s | 20.11s | 70.803s | 13.360s | 83.602s |
| warm C3 | `run-1632318-1784328010349190567` | 195.875s | 87.726s | 16.68s | 70.714s | 18.326s | 83.941s |

The clean median was 359.204s and the warm median was 197.644s. Reuse saved
161.560s/45.0%, clearing both the 15% relative and 30-second absolute
qualification thresholds. Outer-wall medians independently measured 359.38s
clean and 198.06s warm. The current clean median was 8.4% slower than R2-01's
331.422s, while the warm median was 7.7% faster than R2-01's 214.081s. This
host/storage drift changes the absolute endpoints but not the qualification:
R2-01, R2-04, and R2-09 each independently clear both thresholds.

The median clean residual was 50.67s of Nextest compilation/linking, 107.625s
of Nextest execution, 16.246s of doctests, and 112.606s in the keystore and
database/parity tail. The warm medians were 21.24s, 71.882s, 15.068s, and
83.941s respectively. Persistence removes meaningful compiler work, but
execution, rustdoc/doctests, live services, and parity remain most of the warm
gate. Maximum resident memory stayed between 957,788 and 958,108 KiB and no
run swapped.

The current changed-source and path observations used the already-warm target
from clean root A and a disposable second worktree. Each mutation was staged
only in that worktree, passed all 13 leaves, and was restored before the next
case.

| Case | Run | Full CI | Nextest | Compile/link | Execution | Doctest | Verification tail |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| documentation-only | `run-1681527-1784328763000097724` | 196.914s | 92.269s | 18.58s | 73.281s | 13.412s | 84.869s |
| leaf `mfm-authored-config` | `run-1696888-1784328969282287031` | 211.876s | 97.885s | 25.70s | 71.783s | 13.410s | 89.768s |
| shared `mfm-ids` | `run-1716194-1784329196847438403` | 265.513s | 124.707s | 52.07s | 72.125s | 13.872s | 92.093s |
| exact commit, second worktree | `run-1656858-1784328401782937748` | 254.160s | 111.635s | 38.96s | 72.316s | 14.675s | 106.649s |

Relative to the 197.644s warm median, the documentation probe was effectively
unchanged, the leaf edit added 14.232s/7.2%, the shared edit added
67.869s/34.3%, and changing only the worktree path added 56.516s/28.6%.
Nextest execution remained within 1.5s across these four observations; the
changed-source and path costs came principally from compilation, linking, and
the serial verification tail rather than reduced coverage or test selection.

Every qualification and invalidation run reported 974/974 Nextest tests,
doctests, all nine Trybuild harnesses, offline and online SQLx, and keystore,
CLI, REST, metadata, and Postgres parity. A post-matrix inventory regenerated
99 test binaries and 974 identifiers. Sorting
`<binary-id>::<test-name>` in byte order with a trailing newline retained the
frozen SHA-256
`e75d6ea039b5507c6f9b89bef89656e31073c02f8f17f74f680fc2bbf0d67f08`.

An unchanged same-path target occupied approximately 10.49 GB with 17,790
files and approximately 1.79 GB of Trybuild artifacts. The second-worktree
run grew root A's target from 10,485,362,688 to 10,889,363,746 bytes and its
file count from 17,790 to 19,789. After all probes, its Trybuild subtree was
2,229,363,182 bytes. Thus an exact source revision at another path still
accumulates path-specific variants even though most artifacts are reused.

Three fresh roots are the supported cacheless observations. The local target
has no remote backend, so backend-unavailable behavior is not applicable; it
also has no explicit same-namespace bypass. R2-04's failure evidence remains
applicable under the unchanged model, runtime ABI, and cache identity:
active-process cleanup fails closed, cancellation leaves a recoverable Cargo
target, same-target writers rely on Cargo locking rather than a Nixfied lease,
and independently chosen state roots can still collide on slot-derived ports.
R2-09 did not repeat those destructive probes merely to reproduce unchanged
identity evidence.

After evidence extraction, supported slot cleanup removed roots B, A, and C
under cleanup IDs `cleanup-1681368-1784328743439736227`,
`cleanup-1749539-1784329648543288621`, and
`cleanup-1749800-1784329651071822662`. Filesystem availability rose from
approximately 16 GB before cleanup to 39 GB, no experimental slot remained,
and the disposable worktree was removed cleanly. As defined by the current
coarse cleanup contract, the cited run logs were deleted with their slots.

### Normalized candidate comparison

Values below distinguish qualification distributions from screening-only
observations. A dash means the candidate stopped before that measurement; it
does not mean a passing result.

| Candidate | Full-gate distribution | Leaf/shared edit | Local reuse/realization | Persistent bytes | Coverage | Operations | Verdict |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Scoped Cargo target | qualified: clean 320.486-422.086s, median 359.204s (n=3); warm 194.250-244.187s, median 197.644s (n=5); saves 161.560s/45.0% | full CI 211.876s/265.513s; compile 25.70s/52.07s | native same-path Cargo reuse; clean root is the only bypass proxy; no backend | approximately 10.49 GB unchanged; exact second path added approximately 404 MB | exact 99 binaries/974 IDs and all public leaves | slot-wide cleanup only; no cache inspection, bounds, cache-only GC, explicit bypass, enforced worktree owner, or Nixfied writer lease | performance-qualified; operational hold for R2-10 |
| `sccache` | screening only: empty-cache median 357.74s (n=2), warm median 217.72s (n=3 heterogeneous cases); clean regressed 17.4% | focused tests 85.13s/128.70s; 250/66 and 59/363 hits/misses | 100% same-path fresh-target hits, but second path fell to 81.2% and only 12.7% faster than control | 1.03-1.54 GB observed under 5 GiB bound; 32 MiB stress bound evicted useful entries | exact coverage and tested false-hit recovery passed | daemon and explicit wrapper bypass exist; cache remains outside Nixfied ownership, trust, inspection, and lifecycle | rejected at screening: clean regression and path-sensitive materiality failure |
| Crane artifact | screening only: retained artifact/clean residual 271.36s outer; warm 188.81-195.74s, median 195.20s (n=3), only 4.4%/9.05s faster | archive rebuild 54.86s/52.58s; execution 103.18s/81.21s | exact Nix store hits; cold dependency, archive, and execution chain approximately 358s | approximately 442 MB dependency + 758 MB archive + 1.76 GB mutable Trybuild target per execution slot | exact 99/974 inventory; Trybuild remains nested Cargo; doctest and parity remain residual | strong Nix inspection/GC for immutable outputs; archive extraction and mutable residual need separate capacity/lifecycle | rejected at screening: misses full-gate materiality |
| `crate2nix` artifacts | no full gate; hard coverage veto before timing qualification | shared edit changed 263/1,724 aggregate closure paths; leaf artifact reuse alone passed | exact Nix reuse for simple leaf; all-root build took 15:34.87 | leaf tests 15.0 MB output/74.3 MB closure; aggregate closure 733.0 MB plus unrealized full test surface | failed repository/sibling source inputs and supported reusable Trybuild artifact | committed 698 KB graph needs drift gate, or expensive IFD; private API would still not repair semantics | rejected at screening: no authoritative artifact or exact coverage |
| `cargo2nix` artifacts | no full gate; dependency/feature and coverage vetoes before timing qualification | every tracked edit, including README, invalidated every reachable local MFM crate | exact Nix reuse for simple leaf; generator bootstrap 12:22.42 and 1.44 GB unique closure | leaf library/test closures 74.9/61.8 MB; partial all-test graph had 3,029 closure paths | failed Cargo dependency fidelity, 35 test crates, Trybuild, doctests, and parity | 488 KB generated graph has lock drift guard, but older generator/toolchain, overlay order, and overrides add maintenance | rejected at screening: incorrect graph and incomplete coverage |
| Execution topology | no qualifying full-gate distribution; four workers saved 1.6%/1.264s, doctest overlap 4.9%/4.64s, execution-only parity overlap 18.0%/9.05s | n/a | n/a | duplicate execution target reached approximately 11 GB; shared target serializes Cargo writers | focused probes retained exact selected tests; public graph unchanged | true Cargo overlap requires separate targets; current shared target prevents nominal graph concurrency | rejected at screening: every end-to-end saving missed 15% and 30s |

The scoped Cargo target is the sole performance-qualified candidate. At R2-09,
the report interpreted its operational properties as missing Nixfied cache
requirements. The table preserves that then-current interpretation alongside
the measurements. The final responsibility correction below rejects it:
compiler-artifact placement and lifecycle belong to Cargo/MFM, while the
independently observed host-port defect belongs to Nixfied.

Every alternative has an independent screening veto, so the evidence gives no
approved additive hypothesis and no basis for a combined pilot. The comparison
is decision-quality without introducing another compiler cache.

## R2-10 follow-up: MFM Rust build architecture decision

Phase: R2-10

Outcome: passed; owner approved on 2026-07-18

R2-10 selects the existing Cargo/Nix/Nixfied boundary and adds no new compiler
cache. The complete accepted decision is
[ADR 0001](adr/0001-mfm-rust-build-architecture.md). The RFC executive decision,
candidate table, answered questions, selected outcome, gate governance, and
R2-10 stop condition now reflect that architecture.

### Changed and deliberately unchanged

- Development remains direct incremental Cargo in the Nix-pinned environment
  with a worktree-local target.
- Broad local verification retains the compact Cargo policy in the
  worktree-owned `target/verification` directory.
- Release packaging remains the Nix `buildRustPackage` output and is not used
  as a verification cache.
- No `sccache`, Crane archive, crate2nix/cargo2nix graph, remote backend,
  execution-topology change, cache cleaner, or rollout implementation was
  added.
- Current gate policy remains active. Exact-candidate `.#ci` equivalence is a
  selected later simplification, not permission to skip the component gates
  before the repository policy changes.

### Verification

The decision documents passed the current authoritative gates:

| Gate | Run | Result |
| --- | --- | --- |
| `nix run .#check` | `run-2938609-1784374082289985792` | 4/4 leaves passed |
| `nix run .#test` | `run-2938958-1784374094314973495` | 974/974 Nextest tests plus doctests passed |
| `nix run .#test-db` | `run-2953458-1784374210968832990` | 6/6 database/parity leaves passed |
| `nix run .#ci` | `run-2956293-1784374311276794236` | 13/13 leaves passed |

R2-10 introduced no build or runtime implementation and performed no new
performance measurement. It consumes the R2-09 qualification distribution,
the frozen 99-binary/974-test identifier inventory, and the independent
R2-03 through R2-08 screening decisions.

### Acceptance and operational contract

The selected verification target is performance-qualified only on the
reference aarch64-linux host. One worktree owns one
`target/verification`, shared across its Nixfied slots. Cargo owns writer
locking, fingerprints, and rebuild decisions; MFM owns placement, inspection,
retention, cleanup, bypass, and corruption recovery. These are ordinary
project operations, not Nixfied cache-family semantics.

Mutable artifacts are local same-user accelerators, never authoritative
outputs or remotely trusted inputs. Nixfied owns service/process lifecycle,
its registry and state, endpoint coordination, and execution evidence.

### Risks and limitations

- Storage remains project-owned and unbounded unless MFM applies ordinary
  worktree retention or `cargo clean --target-dir target/verification`.
- Hosted Linux and macOS retain correctness coverage, but neither environment
  is performance-qualified by this RFC.
- Cold full verification remains several minutes, stable targets occupy about
  10.49 GB per active worktree, and execution/services dominate the warm gate.
- A remote cache would introduce unmeasured transport, credential, poisoning,
  and failure contracts and therefore requires a separate decision.

The cache-capability recommendation that originally followed was superseded by
the responsibility correction below.

## Final responsibility correction and adoption

The R2-11 handoff was transmitted as a forcing case, but its cache requests
were rejected because they would make the generic runtime own
invocation-specific compiler artifacts. The handoff document is deleted. This
does not invalidate any R2 timing, size, coverage, invalidation, or failure
measurement; it changes the architectural conclusion drawn from them.

The implemented boundary is:

- broad gates set ordinary `CARGO_TARGET_DIR=target/verification`;
- direct development and `.#quick` leave `CARGO_TARGET_DIR` unset;
- all slots in one worktree share the verification target, while separate
  worktrees isolate naturally by path;
- Cargo/MFM own writer locking, fingerprints, inspection, retention, cleanup,
  bypass, and corruption recovery;
- `NIXFIED_STATE_DIR` selects only runtime state/evidence and neither selects
  nor cleans Cargo artifacts;
- Nixfied output carries execution evidence, never compiler-cache evidence;
  and
- hosted CI remains cold/ephemeral unless a separate provider-cache decision
  is made.

The exact project cleanup operation is:

```sh
cargo clean --target-dir target/verification
```

Nixfied still owns the independently measured endpoint problem. Its accepted
endpoint acquisition contract coordinates same-user starts across runtime
roots, verifies exact listener ownership, and returns actionable typed
conflicts before service-specific mutation. This fixes the port defect without
creating a Cargo cache protocol, new semantic authority, or migration path.

No new performance claim is made by this correction. The R2-09 distribution
remains the evidence for retaining native Cargo reuse; future changes to target
placement or hosted persistence require new MFM measurements.
