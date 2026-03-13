# MFM Development Guide for AI Agents

This document is the source-of-truth for AI agents working in this repository.
It is inspired by the practices used in large Rust codebases: modular crates, strong typing, careful performance work, and a bias toward small, reviewable changes.

## Read First (Non-Negotiables)

- Keep changes small and local; prefer 1 logical change per PR/commit.
- Match CI (Nixfied): use `nix run .#check`, `nix run .#test`, and `nix run .#ci -- --mode <mode> --summary`.
- Default pre-commit gate: run `nix run .#ci -- --mode full` before every commit.
- Write commit subjects in lower case. Examples: `mfm-core bump to 0.1.30`, `fix nix task wrappers to preserve caller cwd`, `implement phased publish-docs reconciler`, `docs: publish umbrella earlier with live links only`, `docs: point crate metadata at mfm repo`.
- Never log, print, or persist secrets (passwords, mnemonics, private keys).
- Preserve crate boundaries: libraries stay usable without the CLI.
- Keep binaries (`bin/cli`, `bin/rest-api`) thin:
  - op planning logic belongs in `crates/ops/*-op`
  - reusable executable state logic belongs in shared-state crates (`crates/states/common`, `crates/states/keystore`, `crates/states/aave-v3`, `crates/evm-runtime`)
  - binaries should parse input, start/resume runs, and render outputs only
- Apply the three-tier thin-layer principle from `docs/architecture.md`; use `docs/ops-and-states.md` for the current ops/state catalog:
  - executable logic lives in reusable states (`crates/states/common/src/states/*` and domain shared-state crates)
  - ops stay thin and assemble state graphs
  - binaries stay transport-only
- If you touch security-sensitive code (keystore/crypto), add or strengthen tests.

## Design Contract (Architecture Invariants)

- `docs/redesign.md` is the design contract. If code disagrees with it, the code is wrong (until the doc is updated).
- `docs/architecture.md` is the contributor-facing one-pager.
- `docs/ops-and-states.md` is the current inventory of registered ops and production state implementations.

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
- `nix run .#ci -- --mode basic --summary`
- `nix run .#ci -- --mode audit --summary`
- `nix run .#ci -- --mode parity --summary`
- `nix run .#ci -- --mode full --summary`

## Key docs:

- `README.md`: project disclaimer.
- `docs/repo-map.md`: Repository map
- `docs/architecture.md`: one-page architecture overview + invariants.
- `docs/redesign.md`: full design contract (authoritative).
- `docs/ops-and-states.md`: current inventory of registered ops and production state implementations.
- `bin/cli/README.md`: CLI behavior and JSON output contract.
- `crates/machine/README.md`: state machine concepts and usage.
- `crates/machine-derive/README.md`: proc-macro notes.


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
nix run .#ci -- --mode audit --summary
```


## Nixfied Customization Surface

Nixfied is vendored under `nixfied/`. Vendoring boundaries (canonical doc: `nixfied/VENDORED.txt`):

- Framework-owned (overwritten on `framework::upgrade`): `flake.nix`, `flake.lock`, `nixfied/framework/`.
- User-owned (preserved on `framework::upgrade`): `nixfied/project/` (primary customization surface) and `nixfied/local/` (extensions).

Prefer editing `nixfied/project/` and `nixfied/local/` (not `flake.nix` or framework code under `nixfied/framework/`) for workflow changes:

- `nixfied/project/conf.nix`: project identity, env vars, envs/ports, module toggles, slot behavior.
- `nixfied/project/module.nix`: `nix run .#dev`, `nix run .#mfm_cli`, `nix run .#mfm_rest_api`, `nix run .#build`, `nix run .#check`, `nix run .#test`, and `nix run .#ci` task/workflow wiring including `mfm::portfolio::snapshot`.
- `nixfied/project/ci-runtime.nix`: CI runtime environment helpers and command assembly used by the CI task/workflow layer.
- `nixfied/project/model-introspection.nix`: generated model export/introspection surfaces used by `nix run .#model`, `.#tasks`, and related tooling.
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
- Avoid cloning in hot paths. Prefer `&str`/borrowing where possible. 

### Error Handling

- Libraries:
  - Prefer typed errors (e.g. `thiserror`) for stable, testable behavior.
  - Use `anyhow` primarily for glue code or when error typing adds little value.
- CLI:
  - Preserve stable, machine-readable error codes (see `bin/cli/README.md`).
  - Avoid breaking the JSON output schema.


### Op vs State Placement Contract

- `Operation` (`expand`): planning-only (`config -> graph`), deterministic, no ambient IO.
    - If code builds `StateNode`/`DependencyEdge`, place it in an op crate.
- `State` (`handle`): execution-only (runtime behavior through context + `IoProvider` + recorder).
    - If code implements `State::handle`, place it in a shared-state crate unless it is a justified op-local output/aggregation state.

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
- Commands should be wrappers over run execution:
  - map args -> op/pipeline input
  - start/resume run
  - render final report
  - avoid embedding domain execution logic in CLI command/support modules

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
- Avoid growing a single source file with both large implementation and very large `#[cfg(test)]` blocks.
- Keep only small, local smoke tests inline when they materially improve readability near the code under test.

If a bug can reappear, write a regression test.

## What To Avoid

- Introducing new dependencies without strong justification.
- Changing Public API input/output schemas casually.
- Storing secrets in repo files, logs, fixtures, or test snapshots.

## Performance Considerations

- Avoid accidental allocations in hot paths (crypto, parsing, tight loops).
- Prefer borrowing (`&[u8]`, `&str`) to cloning, especially for large buffers.
- In async code, do not block the runtime; use `tokio::task::spawn_blocking`.
- Be mindful of context scope (avoid holding mutable context across `.await`).

## Documentation Updates Required

When changing CLI/REST behavior, update the relevant docs in the same change:

- `bin/cli/README.md` for CLI contract/usage changes.
- `bin/rest-api/README.md` for REST surface/contract changes.
- `docs/architecture.md` if architectural boundaries or responsibilities change.
- `docs/redesign.md` if runtime/storage/replay contract semantics change.
- Public library API changes must update rustdoc in the same change.
- Keep `#![warn(missing_docs)]` enabled in library crates; new library crates should add it from the start.
- New public items must include rustdoc on the item and its public fields or methods.
- If a crate's main entrypoint or usage changes, update or add at least one rustdoc example in the same change.
- Do not treat documentation as follow-up work for public APIs.

## Debugging Tips

- Backtraces: set `RUST_BACKTRACE=1` (CI already does).
- CLI logging: use `tracing::{debug, info, warn, error}` with a clear target.
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
