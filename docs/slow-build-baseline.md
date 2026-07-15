# Controlled slow-build baseline

Status: Phase 01 baseline for RFC_SLOW_BUILDS.md

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
