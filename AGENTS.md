# MFM Development Guide for AI Agents

This document is the source-of-truth for AI agents working in this repository.
It is inspired by the practices used in large Rust codebases: modular crates, strong typing, careful performance work, and a bias toward small, reviewable changes.

## Read First (Non-Negotiables)

- Keep changes small and local; prefer 1 logical change per PR/commit.
- Follow `docs/code-quality.md` for every code, test, documentation, build, and workflow change.
- Do not introduce hacks, monkey patches, partial workarounds, or fragile compatibility shims.
- If the requested change needs missing underlying support, add that support properly or report the blocker honestly.
- Use focused Cargo verification by default. Prefer targeted `cargo test`, `cargo check`,
  `cargo metadata`, namespace scans, schema checks, and manually started service parity tests.
- Do not use Nixfied test/CI wrappers as the default gate. Use Nixfied commands only when the
  change is specifically about Nixfied behavior or the user asks for them.
- Write commit subjects in lower case. Examples: `mfm-core bump to 0.1.30`, `fix nix task wrappers to preserve caller cwd`, `docs: refresh repo map for typed crates`, `docs: publish umbrella earlier with live links only`, `docs: point crate metadata at mfm repo`.
- Never log, print, or persist secrets (passwords, mnemonics, private keys).
- Preserve crate boundaries: libraries stay usable without the CLI.
- Keep binaries (`bin/cli`, `bin/rest-api`) thin:
  - op planning logic belongs in `crates/ops/*-op`
  - reusable executable state logic belongs in `crates/states/*`
  - runner bindings from state intent to capabilities belong in `crates/adapters/*`
  - live protocol implementations belong in `crates/transports/*`
  - binaries should parse input, start/resume runs, and render outputs only
- Apply the taxonomy and boundary contract from `docs/architecture.md`:
  - operations stay deterministic and assemble state graphs
  - states own reusable domain semantics
  - adapters bind state intent to explicit capabilities
  - transports and signers stay reusable platform primitives
  - binaries stay transport-only
- If you touch security-sensitive code (keystore/crypto), add or strengthen tests.

## Design Contract (Architecture Invariants)

- `docs/design.md` is the design contract. If code disagrees with it, the code is wrong (until the doc is updated).
- `docs/architecture.md` is the contributor-facing taxonomy and placement guide.

Key invariants to preserve (high risk if violated):

- Append-only stream families, with `run:*` carrying machine events (no mutation of past records).
- Per-append atomicity in stream stores: each append is all-or-nothing.
- Content addressing for manifests, snapshots, facts, and outputs.
- Canonical JSON for hashing structured data (target semantics: RFC 8785 / JCS-style).
  - Hashed structures MUST NOT contain floats (use integer-scaled values or decimal strings).
- No ambient IO in state logic: route network/FS through an IO abstraction that supports live and replay.
- Secrets must not appear in persisted surfaces:
  - manifests, events, artifacts (including fact payloads and context snapshots), CLI/API outputs, or error details.

## Verification Entry Points

Use Cargo and focused checks first:

- `cargo fmt --all -- --check`
- `cargo check --workspace`
- `cargo test --workspace`
- `cargo test -p mfm-integration-tests --test cargo_metadata_contract`
- `cargo test -p mfm-integration-tests --test architecture_namespace_contract`
- `rg` namespace/config scans for architecture guardrails

For parity tests that need Postgres, Reth, or other live services, start those services manually and
run the focused Cargo test with explicit environment variables such as `DATABASE_URL`,
`RETH_HTTP_PORT`, or `MFM_EVM_RPC_SOURCES_JSON`.

## Key Docs:

- `README.md`: project disclaimer.
- `docs/repo-map.md`: Repository map
- `docs/code-quality.md`: mandatory quality policy for all changes.
- `docs/architecture.md`: taxonomy, placement rules, and architecture boundaries.
- `docs/design.md`: full design contract (authoritative).
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

1. **Formatting**:
```bash
cargo fmt --all -- --check
```

2. **Focused testing**:
```bash
cargo test -p <package>
```

3. **Workspace testing when feasible**:
```bash
cargo test --workspace
```


## Nixfied Customization Surface

Nixfied is vendored under `nixfied/`. Vendoring boundaries (canonical doc: `nixfied/VENDORED.txt`):

- Framework-owned (overwritten on `framework::upgrade`): `flake.nix`, `flake.lock`, `nixfied/framework/`.
- User-owned (preserved on `framework::upgrade`): `nixfied/project/` (primary customization surface) and `nixfied/local/` (extensions).

Prefer editing `nixfied/project/` and `nixfied/local/` (not `flake.nix` or framework code under `nixfied/framework/`) for workflow changes:

- `nixfied/project/conf.nix`: project identity, env vars, envs/ports, service defaults, slot behavior.
- `nixfied/project/module.nix`: modeled task/workflow wiring for dev, REST API, build,
  check/test, CI, and portfolio snapshot app surfaces.
- `nixfied/project/ci-runtime.nix`: CI runtime environment helpers and command assembly used by the CI task/workflow layer.
- Framework introspection surfaces such as `.#introspect`, `.#schema`, `.#features`, and package outputs like `.#introspectionBundle`: compiled model surfaces that project CI checks should consume directly.
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
cargo run -p mfm -- --help
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
- `docs/design.md` if runtime/storage/replay contract semantics change.
- Public library API changes must update rustdoc in the same change.
- Keep `#![warn(missing_docs)]` enabled in library crates; new library crates should add it from the start.
- New public items must include rustdoc on the item and its public fields or methods.
- If a crate's main entrypoint or usage changes, update or add at least one rustdoc example in the same change.
- Do not treat documentation as follow-up work for public APIs.

## Debugging Tips

- Backtraces: set `RUST_BACKTRACE=1` (CI already does).
- CLI logging: use `tracing::{debug, info, warn, error}` with a clear target.
- For parity failures, keep service logs beside the manually started Postgres/Reth data directory
  and rerun the focused Cargo test with `-- --nocapture`.


## Commenting Guidelines (Keep Future Readers in Mind)

Write comments that remain valuable after the PR merges.

Good comments explain WHY / constraints / invariants:

```rust
// AAD binds ciphertext to entry id, preventing swap attacks.
// Any change here must remain constant-time.
```

Avoid comments that just restate code or describe the PR.

`unsafe` blocks must always be documented.
