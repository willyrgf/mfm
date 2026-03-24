# RFC: Vendored Project And Offline Cache Workflows

Status: draft

Last updated: 2026-03-22

## Summary

We want two related but different capabilities:

1. A default workflow for offline and reproducible local work that does not require checking all upstream sources into git.
2. An explicit `.#vendor-project` command that can produce a portable bundle which runs with zero network fetches.

This RFC proposes:

- The default path should be an offline cache workflow, not in-repo vendoring.
- `.#vendor-project` should produce a bundle directory containing source snapshots, a local Nix binary cache, Cargo-vendored crates, and optional extra snapshots such as the RustSec advisory DB.
- We should not try to vendor all of `nixpkgs` into the repository.

## Problem Statement

Today the repository is partially vendored, but not fully offline:

- The Nixfied framework itself is vendored under `nixfied/`.
- Flake inputs are still remote:
  - `flake.nix`
  - `flake.lock`
- Rust dependencies are still remote crates.io sources locked through `Cargo.lock`.
- Extra remote source fetches exist in Nix:
  - `nixfied/project/aave-origin-tools.nix`
  - `nixfied/framework/runtime/services/helios/package.nix`
- Many user-facing tasks run `cargo` directly instead of only consuming prebuilt Nix packages:
  - `nixfied/project/module.nix`

That means:

- A warm `/nix/store` alone is not enough for offline development.
- A source-vendored repository alone is not enough for a fresh machine, because Nix packages and toolchains still need closures.

## Current State

### Already vendored

- Nixfied framework source is vendored under `nixfied/`.
- The project wrapper is already a vendored Nixfied wrapper.

### Still remote

- Flake inputs:
  - `github:NixOS/nixpkgs/nixos-unstable`
  - `github:numtide/flake-utils`
- Rust crates from crates.io, as recorded in `Cargo.lock`.
- Aave V3 origin source via `pkgs.fetchFromGitHub`.
- Helios source via `fetchFromGitHub`.

### Important Cargo behavior

The repo does not currently have a workspace `.cargo/config.toml`.

User-facing commands such as these run `cargo` directly:

- `nix run .#build`
- `nix run .#check`
- `nix run .#test`
- `nix run .#dev`
- `nix run .#mfm_cli`
- `nix run .#mfm_rest_api`

So any offline story must handle both:

- Nix-level fetching and closure availability
- Cargo-level dependency source resolution

### CI shape

Relevant current workflows:

- `workflow.ci.full` = basic + parity
- `workflow.ci.audit` is separate
- `workflow.ci.mainnet` is separate

This matters because the default offline target set does not need to include every workflow.

## Goals

- Support offline local development after an initial connected preparation step.
- Support a stricter "zero network fetches" bundle for transfer to another machine.
- Preserve reproducibility through lock files and explicit snapshots.
- Avoid bloating the git repository with large vendored upstream trees.
- Keep the workflow compatible with Nixfied task and app entrypoints.

## Non-Goals

- Vendoring all of `nixpkgs` into git.
- Making mainnet runtime behavior itself networkless.
- Replacing the Nix store model with only git-tracked source vendoring.
- Redesigning the Nixfied launcher architecture as part of the first implementation.

## Key Distinction

This RFC separates two different ideas:

- Source vendoring: copying upstream source trees locally
- Closure export: making already-resolved Nix store paths available offline

For Nix-based projects, true offline execution generally requires both.

## Proposal

### 1. Default workflow: offline cache preparation

Tentative command name:

- `nix run .#cache-project`

This should be the default supported path for day-to-day work.

#### Desired behavior

Run on a connected machine and prepare a local offline asset set that can later be reused without network access.

It should:

- Archive flake inputs into the local store and export them into a local Nix binary cache.
- Realize and export the closures needed for selected project apps and workflows.
- Vendor Cargo dependencies into a local directory derived from `Cargo.lock`.
- Generate Cargo source-replacement config that points to the vendored directory.
- Optionally snapshot the RustSec advisory DB for offline `cargo audit`.
- Write a small manifest or README describing what was prepared.

#### Default target set

The default warmed surface should be practical, not maximal.

Suggested default targets:

- `dev`
- `build`
- `check`
- `test`
- `mfm_cli`
- `mfm_rest_api`
- `ci --mode full`

Suggested opt-in targets:

- `ci --mode audit`
- `ci --mode mainnet`
- custom app and workflow selections

#### Why this should be the default

- Matches how Nix wants to distribute build results: via store paths and caches.
- Keeps the repository small.
- Avoids committing large generated trees.
- Can be refreshed incrementally as locks or sources change.

### 2. Explicit workflow: vendored project bundle

Tentative command name:

- `nix run .#vendor-project`

This should be the stricter portability command.

#### Desired behavior

Produce a bundle directory that can be moved to another machine and used without network fetches.

The output should be a bundle, not an in-place mutation of the git repository.

Suggested bundle contents:

- `project/`
  - source snapshot of the repo
- `binary-cache/`
  - local `file://` Nix binary cache containing required closures and flake input store paths
- `cargo-vendor/`
  - vendored Cargo dependencies
- `cargo-config/` or bundle-local `.cargo/config.toml`
  - source replacement pointing Cargo to vendored deps
- `advisory-db/`
  - optional snapshot for offline `cargo audit`
- `nix.conf`
  - bundle-local Nix config pointing to the local binary cache
- `README.md` or `run.sh`
  - one-file instructions for offline usage

#### Why bundle output is preferred over in-repo vendoring

- Vendoring `nixpkgs` into git is too large and awkward to review.
- Nix packages and toolchains are naturally represented as store closures, not committed source trees.
- A movable bundle gives us the zero-fetch property we want without permanently inflating the repo.

## Why not vendor everything into git?

Because that solves the wrong layer.

Even if we copied the following into the repository:

- flake inputs
- Cargo crates
- Helios source
- Aave origin source

We would still need Nix package closures for:

- toolchains
- compilers
- shell tools
- runtime services
- transitive package dependencies

So source vendoring alone does not yield a complete offline system on a fresh machine.

## Implementation Notes

### Nix side

The Nix part of the solution should use:

- lock-file-based flake input resolution
- flake input archiving
- local `file://` binary cache export
- closure copying for the selected build and runtime surfaces

The implementation should target real app closures, not only the thin launcher scripts.

### Cargo side

The Cargo part of the solution should use:

- `cargo vendor`
- generated source replacement config
- offline mode when using vendored sources

This is important because current project tasks invoke `cargo` directly.

### Audit side

`cargo audit` needs a local advisory database snapshot if we want audit to be usable offline.

This should be opt-in for the first pass.

## Nixfied Assessment

### What Nixfied already provides

- Vendored framework wrapper install and upgrade flows:
  - `framework::install`
  - `framework::upgrade`

### What it does not currently provide

- A project-wide "vendor downstream repo" command
- A stable surface for materialized runtime app derivations
- A generic local binary cache export helper for downstream projects

### MVP conclusion

We do not need Nixfied framework changes to build the first working version in this repository.

The first version can be repo-local.

### Small framework improvements that would help

These look useful, but not mandatory for the MVP:

1. Expose materialized runtime app derivations as a stable package surface, for example:
   - `legacyPackages.<system>._nixfied.runtimeApps.<app>`
2. Add a helper or preset for exporting flake input and app closures into a local binary cache.
3. Add a helper for enumerating the closure set implied by a task or workflow family.

## Open Questions

1. Naming:
   - `cache-project` vs `offline-cache` vs `prefetch-project`
   - `vendor-project` vs `bundle-project`
2. Output layout:
   - should offline assets live under `.offline/`, `dist/`, or another ignored directory?
3. Scope:
   - should `vendor-project` include `ci audit` by default, or keep RustSec DB optional?
4. Fresh-machine UX:
   - should the bundle include a wrapper script that sets the right Nix options automatically?
5. Closure targeting:
   - what is the cleanest way to realize the actual runtime app closures behind launcher apps?
6. Policy:
   - do we want a future `--in-place` mode, or should vendoring always mean bundle output?

## Draft Decision

The current draft decision is:

- Default support should be offline cache preparation, not git vendoring.
- `.#vendor-project` should produce a portable bundle, not a massive in-repo vendored tree.
- Nixfied framework changes are useful but not required for the first implementation.

## Proposed Phases

### Phase 1

- Land this RFC.
- Implement a repo-local prototype for the default offline cache workflow.

### Phase 2

- Implement `.#vendor-project` as a portable bundle generator.

### Phase 3

- Upstream the smallest reusable Nixfied improvements if the repo-local prototype proves the shape.

## File Anchors

Relevant files for this RFC:

- `flake.nix`
- `flake.lock`
- `Cargo.lock`
- `nixfied/VENDORED.txt`
- `nixfied/project/conf.nix`
- `nixfied/project/module.nix`
- `nixfied/project/aave-origin-tools.nix`
- `nixfied/framework/runtime/services/helios/package.nix`
- `nixfied/framework/install/wrapper-command.nix`
- `nixfied/framework/install/wrapper-flake.nix`
- `nixfied/framework/core/mkFlakeOutputs.nix`

## External References

- Nix flake reference manual
- Nix local binary cache store manual
- Cargo `vendor` command documentation
- Cargo source replacement configuration documentation
- `cargo-audit` advisory database configuration docs

## Discussion Notes

This file is intended to be edited iteratively.

Expected next edits:

- tighten command names
- choose output directory layout
- define the first implementation target set
- decide whether audit support is in or out of the first pass
