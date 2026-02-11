# MFM Development Guide for AI Agents

This document is the source-of-truth for AI agents working in this repository.
It is inspired by the practices used in large Rust codebases: modular crates, strong typing, careful performance work, and a bias toward small, reviewable changes.

## Read First (Non-Negotiables)

- Keep changes small and local; prefer 1 logical change per PR/commit.
- Match CI (Nixfied): use `nix run .#check`, `nix run .#test`, and `nix run .#ci -- --basic/--audit/--parity --summary`.
- Never log, print, or persist secrets (passwords, mnemonics, private keys).
- Preserve crate boundaries: libraries stay usable without the CLI.
- If you touch security-sensitive code (keystore/crypto), add or strengthen tests.

## Design Contract (Architecture Invariants)

- `REDESIGN.md` is the design contract. If code disagrees with it, the code is wrong (until the doc is updated).
- `ARCHITECTURE.md` is the contributor-facing one-pager.

Key invariants to preserve (high risk if violated):

- Append-only event streams (no mutation of past events).
- Per-append atomicity in event stores: each append is all-or-nothing.
- Content addressing for manifests, snapshots, facts, and outputs.
- Canonical JSON for hashing structured data (target semantics: RFC 8785 / JCS-style).
  - Hashed structures MUST NOT contain floats (use integer-scaled values or decimal strings).
- No ambient IO in state logic: route network/FS through an IO abstraction that supports live and replay.
- Secrets must not appear in persisted surfaces:
  - manifests, events, artifacts (including fact payloads and context snapshots), CLI/API outputs, or error details.

## Nixfied Entry Points

Nixfied is the canonical entrypoint for dev/test/build/check/ci:

- `nix run .#help`
- `nix run .#dev`
- `nix run .#check`
- `nix run .#test`
- `nix run .#build`
- `nix run .#ci -- --basic --summary`
- `nix run .#ci -- --audit --summary`
- `nix run .#ci -- --parity --summary`

## Project Overview

MFM is a WIP toolkit for on-chain operations.

What exists today:

- A security-hardened Ethereum keystore (persisted JSON, tamper detection, DoS guards).
- A generic async recoverable state machine for multi-step workflows.
- A scriptable CLI that currently exposes keystore management.
- YAML configuration models for networks/tokens/DEXes (not fully wired end-to-end yet).

Status: experimental, not production-ready, and not intended for mainnet use.

## Repository Map

Workspace root: `Cargo.toml`

- `crates/core/`: core library (keystore + config models + primitives). Security-critical.
- `crates/machine/`: state machine runtime + recovery/replay primitives.
- `crates/machine-derive/`: proc-macro derive crate for compile-time ergonomics.
- `crates/sdk/`: integration glue (op registry + run launch/resume helpers).
- `crates/ops/`: domain operations (expand to state graphs).
- `crates/collectors/`: data collection/normalization libraries (HTTP/RPC/etc).
- `crates/storages/`: persistence backends (event stores, artifact stores, indexers).
- `bin/cli/`: CLI package `mfm` (bin `mfm_cli`).
- `bin/rest-api/`: REST API package `mfm-rest-api` (bin `mfm_rest_api`).
- `nixfied/`: Nixfied framework integration (commands, CI DSL, module apps).

Key docs:

- `README.md`: project disclaimer.
- `ARCHITECTURE.md`: one-page architecture overview + invariants.
- `REDESIGN.md`: full design contract (authoritative).
- `bin/cli/README.md`: CLI behavior and JSON output contract.
- `crates/machine/README.md`: state machine concepts and usage.
- `crates/machine-derive/README.md`: proc-macro notes.

## Architecture Overview

### Core Components

1. `crates/machine/`: execution model
   - state traits + metadata
   - context + snapshots
   - execution planning + runtime (recovery/replay aware)
2. `crates/storages/*`: persistence backends
   - event stores (append-only, atomic append semantics)
   - artifact stores (content-addressed blobs/documents)
3. `crates/ops/*`: domain workflows
   - ops expand into state graphs, emit domain events, store artifacts/facts
4. `crates/core/`: primitives + security-sensitive components
   - `mfm_core::keystore`: encrypted key storage and signing utilities
   - `mfm_core::config`: YAML config models
5. `bin/cli/`, `bin/rest-api/`: thin wrappers
   - parse requests, start/resume runs, render stable outputs

Crate dependency graph:

```text
bin/cli (mfm) -> crates/sdk -> crates/machine
bin/cli (mfm) -> crates/ops/* -> crates/core + crates/collectors/* + crates/storages/*
bin/rest-api (mfm-rest-api) -> crates/sdk -> crates/machine
```

## Key Design Principles

- Modularity: each crate should be usable as a library with minimal coupling.
- Type Safety: strong typing throughout with minimal use of dynamic dispatch
- Explicit boundaries: keep public APIs stable; avoid leaking implementation details.
- Performance as a feature: avoid accidental allocations in hot paths; measure first.
- Extensibility: prefer traits + generic types when it improves composability.
- Correctness and security first: especially in security related modules (e.g. `mfm_core::keystore`).
- Output stability: treat CLI JSON/text formats as public API.

## Development Workflow

### Code Style and Standards

1. **Formatting + Clippy (nightly)**:
```bash
nix run .#check
```

2. **Testing**:
```bash
nix run .#test
```

3. **Security Audit**:
```bash
nix run .#ci -- --audit --summary
```

### Nix (CI Parity)

The repo ships a Nix flake (`flake.nix`). CI runs:

```bash
nix flake check
nix build
nix run .#mfm_cli -- --help
```

## Nixfied Customization Surface

Nixfied is vendored under `nixfied/`. Vendoring boundaries (canonical doc: `nixfied/VENDORED.txt`):

- Framework-owned (overwritten on `framework::upgrade`): `flake.nix`, `flake.lock`, `nixfied/.framework/`.
- User-owned (preserved on `framework::upgrade`): `nixfied/project/` (primary customization surface) and `nixfied/local/` (extensions).

Prefer editing `nixfied/project/` and `nixfied/local/` (not `flake.nix` or framework code under `nixfied/.framework/`) for workflow changes:

- `nixfied/project/conf.nix`: project identity, env vars, envs/ports, module toggles, slot behavior.
- `nixfied/project/dev.nix`: `nix run .#dev`, plus convenience runners like `nix run .#mfm_cli` / `nix run .#mfm_rest_api`.
- `nixfied/project/test.nix`: `nix run .#test` (nextest).
- `nixfied/project/quality.nix`: `nix run .#check` (nightly fmt + clippy).
- `nixfied/project/prod.nix`: `nix run .#build` (release build).
- `nixfied/project/ci.nix`: CI pipeline DSL (modes/steps, artifacts, parity services).
- `nixfied/project/default.nix`: merges project files; update it if you add a new `nixfied/project/*.nix` part.
- `nixfied/local/default.nix`: optional extension point for extra flake `apps`/`packages`/`devShells` that should survive framework upgrades.

Environment variables you should expect:

- `MFM_ENV`: environment name (`dev|test|prod`).
- `NIX_ENV`: slot number (0-9) for disjoint ports when running multiple local instances (defaults to `0` when unset).

## Common Contribution Types

These are typical, review-friendly change patterns (focus on a single outcome).

1. Small bug fixes (1-20 lines)
   - Fix off-by-one / validation edge case
   - Tighten error messages or error variants
   - Add missing `#[serde(default)]` for backward compatibility

2. Security hardening
   - Strengthen keystore tamper checks
   - Add stricter file size/shape validation
   - Reduce secret copies / ensure zeroization

3. Adding comprehensive tests
   - Regression tests for previously failing inputs
   - Corruption/tamper tests for persisted formats
   - CLI e2e tests for command workflows

4. Making components more generic / reusable
   - Prefer traits + bounds over hard-coded types when it improves reuse
   - Keep crate boundaries intact (no `crates/*` -> `bin/*` coupling)

5. Feature additions
   - New CLI subcommand with stable JSON output
   - New state machine scheduler/tracker implementation with tests


## Code Style & API Guidelines

### Rust Style

- Prefer explicit, readable code over cleverness.
- Avoid panics in library code. Use `Result` and typed errors.
- Keep public APIs documented and consistent (names, error behavior, invariants).
- Prefer `&str`/borrowing where possible; avoid cloning in hot paths.

### Error Handling

- Libraries:
  - Prefer typed errors (e.g. `thiserror`) for stable, testable behavior.
  - Use `anyhow` primarily for glue code or when error typing adds little value.
- CLI:
  - Preserve stable, machine-readable error codes (see `bin/cli/README.md`).
  - Avoid breaking the JSON output schema.

### Logging

- Library crates:
  - Prefer `log` to avoid forcing a subscriber on downstream users.
- Binaries:
  - Prefer `tracing` (CLI initializes `tracing_subscriber` in `bin/cli/src/main.rs`).
- Never log secrets (passwords, mnemonics, private keys, raw ciphertext).

### Unsafe

Avoid `unsafe` where possible. If you must use it:

- Explain the safety invariants (why it is safe, what must remain true).
- Add tests that would fail if invariants are violated.

## Domain-Specific Guidance

### Keystore (`crates/core/src/keystore/`)

This is security-sensitive code. Treat changes here as high risk.

Rules:

- Do not introduce secret-bearing debug output.
- Keep secrets in `zeroize::Zeroizing` (or equivalent), minimize copies, and aggressively drop temporary buffers.
- Preserve constant-time comparisons where used (`subtle`).
- Keep the threat model in mind: tamper detection, swap attacks, DoS via file size.
- Do not make `Keystore` `Send`/`Sync` without a deliberate redesign.
- Be careful with format compatibility: keystore files are persisted JSON. If you add fields, use `#[serde(default)]` for backwards compatibility.

When adding features (export, password change, etc.):

- Add unit tests and corruption/tamper tests.
- Prefer explicit, test-backed behavior over implicit "best effort".

### CLI (`bin/cli/`)

The CLI is designed to be scriptable and AI-friendly.

- Respect global `--output-format` and `MFM_OUTPUT_FORMAT`.
- When adding commands, maintain both:
  - human-readable `text` output
  - machine-readable `json` output with stable structure
- Provide non-interactive modes (`--stdin`, `--yes`, env vars) where appropriate.
- Keep outputs stable; treat output format as part of the public API.

Run locally:

```bash
nix run .#mfm_cli -- --help
nix run .#mfm_cli -- keystore list
```

### State Machine (`crates/machine/`)

The state machine is async and uses a typed tag/label system.

- Keep states deterministic unless explicitly tagged as side-effecting.
- Do not block the async runtime:
  - use async I/O where possible
  - use `tokio::task::spawn_blocking` for CPU-bound or blocking work
- Prefer `TypedContextExt::{read_typed, write_typed}` over ad-hoc JSON.
- If you change scheduling/recovery semantics, add tests that assert behavior.

### Proc-Macros (`crates/machine-derive/`)

- Optimize for clear compile-time errors.
- Avoid expanding to surprising code (keep generated impls small and idiomatic).
- Add targeted tests in consuming crates when macro behavior changes.

## Testing Guidance

Prefer tests that lock in behavior at boundaries:

- Unit tests for pure functions and small components.
- Integration tests for interactions between components like:
  - keystore persistence and file integrity
  - CLI command workflows (happy path + failure modes)
  - state machine execution and recovery

Keep implementation files readable by separating large test suites:

- Avoid growing a single source file with both large implementation and very large `#[cfg(test)]` blocks.
- For non-trivial test coverage, move tests to separate files (for example, `mod tests;` with `tests.rs`/`tests/*.rs`, or crate-level `tests/`).
- Keep only small, local smoke tests inline when they materially improve readability near the code under test.

Optional but useful in the right places:

- Property tests for invariants (parsing/validation code).
- Fuzz tests for parsers/decoders/serialization.
- Benchmarks for performance-critical paths (crypto, hot loops, large files).

If a bug can reappear, write a regression test.

## What To Avoid

- Introducing new dependencies without strong justification.
- Changing CLI output schemas casually.
- Storing secrets in repo files, logs, fixtures, or test snapshots.

## Performance Considerations

- Avoid accidental allocations in hot paths (crypto, parsing, tight loops).
- Prefer borrowing (`&[u8]`, `&str`) to cloning, especially for large buffers.
- In async code, do not block the runtime; use `tokio::task::spawn_blocking`.
- Be mindful of context scope (avoid holding mutable context across `.await`).

## Common Pitfalls

- Logging secrets (passwords, private keys, mnemonics, decrypted buffers).
- Breaking persisted formats without `#[serde(default)]` or migration strategy.
- Holding locks across `.await` (can deadlock or stall progress).
- Introducing `unwrap()`/`expect()` in library code where errors should be surfaced.
- Changing CLI output schema or error codes without updating docs and tests.

## CI Requirements

Before opening a PR (or finishing a change), ensure [Code Style and Standards](#code-style-and-standards) are met.

If you use Nix or need CI parity, also run: `nix flake check && nix build`.

## Debugging Tips

- Backtraces: set `RUST_BACKTRACE=1` (CI already does).
- CLI logging: use `tracing::{debug, info, warn, error}` with a clear target.
- Library logging: use `log::{debug, info, warn, error}`.
- Tests: prefer isolated temp dirs/files via `tempfile`.
- CI summaries: `nix run .#ci -- --summary` writes `summary.json` to the artifacts dir (see `nixfied/project/ci.nix`).


## Commenting Guidelines (Keep Future Readers in Mind)

Write comments that remain valuable after the PR merges.

Good comments explain WHY / constraints / invariants:

```rust
// AAD binds ciphertext to entry id, preventing swap attacks.
// Any change here must remain constant-time.
```

Avoid comments that just restate code or describe the PR.

`unsafe` blocks must always be documented.

DO:

```rust
// HashMap provides O(1) lookups during keystore list filtering.
// This avoids repeated linear scans in the CLI hot path.
```

DONT:

```rust
// Changed from Vec to HashMap for O(1) lookups.
```

## Quick Reference

### Essential Commands

```bash
# Quality checks (fmt + clippy, nightly toolchain)
nix run .#check

# Run tests (nextest)
nix run .#test

# Security audit
nix run .#ci -- --audit --summary

# CI modes
nix run .#ci -- --basic --summary
nix run .#ci -- --parity --summary

# Release build (all features)
nix run .#build

# For ad-hoc cargo commands (bench/check/doc), use the pinned dev shell:
nix develop
```
