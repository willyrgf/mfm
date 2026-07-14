# RFC: slow builds

Status: problem statement and investigation

Date: 2026-07-14

## Problem situation

The MFM compile and verification workflow is becoming slow enough to interrupt
normal development. The main issue is not one unusually slow Rust crate. It is
the combined cost of a large workspace, broad Cargo invocations, repeated
feature and target combinations, fragmented build caches, and large debug/test
artifacts.

This RFC records the observed situation and the constraints for a later
optimization pass. It does not select or implement a remediation.

## Evidence

The investigation used the current `mfm3` checkout, Cargo metadata and tree
inspection, the Nixfied task definitions, existing build artifacts, and a warm
timing pass.

| Measurement | Observation |
| --- | --- |
| Workspace | 53 packages and 100 targets |
| Resolved dependency graph | 504 packages and 1,808 dependency edges |
| Build complexity | 71 build-script packages and 33 proc-macro packages |
| Local Cargo target | Approximately 11 GB in `target/` |
| Nixfied Cargo target | Approximately 44 GB in the persistent Nixfied state |
| Nixfied incremental artifacts | Approximately 21 GB and 982 incremental directories |
| Nixfied dependency artifacts | Approximately 19 GB |
| Nixfied trybuild artifacts | Approximately 4 GB |
| Split-debug files | Approximately 179,000 `.dwo` files in the Nixfied target |
| Warm workspace check | `cargo check --workspace --all-targets` completed in 1.77 seconds |

The warm check is not a clean-build benchmark. It shows that the compiler is
fast when the relevant artifacts are already available; it does not represent
the cost of a cold checkout or an invalidated feature/profile combination.

The latest persisted Nixfied CI record under the project state took 440.6
seconds. Its largest task durations were:

- workspace nextest: 112.6 seconds for 983 tests; test execution itself took
  77.3 seconds;
- workspace doc tests: 130.1 seconds;
- the separate `mfm-app` test-support run: 27.7 seconds;
- Clippy: 17.7 seconds.

That record was produced from the sibling `/home/willyrgf.linux/dev/mfm`
checkout at the previous commit. Its main Cargo and Nix configuration files
are byte-identical to this checkout, so it is useful task evidence but should
not be treated as a fresh clean-build benchmark for `mfm3`.

## Current Cargo configuration

The workspace and shared dependencies are defined in
[Cargo.toml](Cargo.toml).

- The workspace uses resolver 2 and includes all 53 packages.
- The shared Tokio dependency enables `features = ["full"]`, so every crate
  using the workspace Tokio dependency receives the complete Tokio feature
  set.
- The development and test profiles only override
  `split-debuginfo = "unpacked"`. They otherwise use the default unoptimized,
  debuginfo-oriented development/test behavior.
- A `[profile.ci]` profile exists with `codegen-units = 256`, but none of the
  Nixfied Cargo commands selects it with `--profile ci`. The observed builds
  therefore report the normal `dev` or `test` profile.
- No repository or user Cargo configuration was found for `build.jobs`,
  `target-dir`, `RUSTC_WRAPPER`, `sccache`, linker selection, or custom
  rustflags.

The dependency graph also contains multiple versions of several transitive
packages, including `digest`, `rand_core`, `sha2`, `toml`, `windows-sys`, and
related platform crates. Some duplication is expected from upstream, but it
increases the amount of code Cargo must resolve and compile.

There are also feature-shape hotspots that deserve measurement:

- Tokio is globally configured with `full` features.
- The BTC and EVM transports use different Reqwest Rustls feature variants.
- `--all-features` enables the complete workspace feature surface during
  Clippy, including test-support and parity-related paths.

## Current Nixfied configuration

The build environment is defined in [nixfied.nix](nixfied.nix).

Each Cargo leaf sets:

```text
CARGO_TARGET_DIR=${stateDir}/cargo-target
RUST_BACKTRACE=1
TMPDIR=${stateDir}
```

The Nixfied child environment starts empty. The tool set includes the pinned
Rust toolchain, Cargo Nextest, Git, `pkg-config`, and `cc`, but no compiler
cache or Rust compiler wrapper.

The direct Cargo target and the Nixfied target are therefore separate caches:

```text
direct cargo:  /home/willyrgf.linux/dev/mfm3/target
Nixfied:      /home/willyrgf.linux/.local/state/nixfied/mfm/dev/0/cargo-target
```

The Nixfied project id is `mfm`, and the persistent state is shared by the
`mfm` and `mfm3` checkouts. This permits reuse across checkouts, but it also
allows old fingerprints, incremental units, and test artifacts to accumulate
or become invalidated by source-path and checkout changes.

The verification tasks are intentionally broad:

- Clippy runs `--workspace --lib --examples --tests --benches --all-features`.
- Nextest runs the complete workspace.
- A second Nextest invocation runs `mfm-app` with `test-support` enabled.
- A third Cargo invocation runs workspace doc tests.
- The SQLx preparation task explicitly runs
  `cargo clean -p mfm-stream-store-postgres` before preparing queries.
- Database parity tasks then compile and run several additional package/test
  combinations sequentially.

This is appropriate as a comprehensive gate, but it is too broad to serve as
the normal inner development loop.

## Test compilation cost

The workspace contains nine trybuild UI-test harnesses. These are valuable
compile-time API and typestate checks, but they compile many small test crates
and currently account for approximately 4 GB under the Nixfied target's
`tests/trybuild` directory. The slowest UI tests in the latest nextest output
took between roughly four and eleven seconds each.

The test graph also contains candidate unused dev-dependencies. `cargo
machete --with-metadata` reported 25 candidate unused direct dependencies,
including `trybuild` in `mfm-state-evm-contracts` and `mfm-signing`, several
CLI test dependencies, and an unused Tokio dependency in
`mfm-transports-proof`. These are signals for a separate verified cleanup;
they are not automatic removal decisions because macro, feature, and test
usage can be difficult for static dependency tools to infer.

## Consequences

The current configuration has the following practical consequences:

1. Switching between direct Cargo commands and Nixfied commands causes work
   to be repeated in different target trees.
2. A small source change can be followed by a broad all-target/all-feature
   build, even when only one package is relevant.
3. Running workspace tests and then the app test-support suite creates another
   feature-specific compilation surface.
4. Split DWARF and incremental compilation preserve useful debugging and
   rebuild behavior, but produce very large numbers of files and substantial
   filesystem metadata work.
5. Explicit package cleaning in the SQLx task invalidates artifacts that later
   parity tasks may need again.
6. Unused dependencies and broad dependency features enlarge the test and
   compile graph without contributing to the resulting binary or test.

## Constraints for remediation

Any future optimization should preserve:

- the crate boundaries and architecture taxonomy;
- deterministic, typed compile-time and trybuild contract checks;
- CLI and REST output contracts;
- security-sensitive debug and artifact redaction behavior;
- the ability to run the full Nixfied gates in CI;
- reproducibility between the pinned Nix toolchain and direct Cargo usage.

Optimizations should be measured separately for clean builds, warm builds,
single-package development checks, and full CI. A faster local profile must not
silently replace the comprehensive verification profile.

## Areas requiring a design decision

The next RFC revision or implementation should decide, with timing evidence,
whether to:

- provide a narrow development task while retaining broad CI gates;
- establish a deliberate cache-sharing strategy between direct Cargo and
  Nixfied;
- add a compiler cache to the hermetic Nixfied tool environment;
- introduce an explicit local/debug profile with a documented debug-info
  tradeoff;
- remove verified unused dependencies and reduce unnecessary feature sets;
- reduce duplicate transitive versions where the upstream dependency graph
  permits it;
- isolate SQLx preparation invalidation from the artifacts used by parity
  tests.

