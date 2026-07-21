# MFM Development Guide for AI Agents

This document is the source of truth for AI-agent behavior and contribution rules in this
repository. `docs/build-and-verification.md` owns workflow mechanics and verification selection;
`nixfied.nix` owns the executable task graph.

## Read First (Non-Negotiables)

- Keep changes small and local; prefer 1 logical change per PR/commit.
- Follow `docs/code-quality.md` for every code, test, documentation, build, and workflow change.
- Do not introduce hacks, monkey patches, partial workarounds, or fragile schema shims.
- If the requested change needs missing underlying support, add that support properly or report the blocker honestly.
- Verification is scope-driven, not commit-driven. Use the narrowest command that exercises the
  changed behavior, and expand only when the affected boundary or risk requires it.
- Do not run broad gates merely because a commit is about to be created. Follow the verification
  ladder below and report exactly what was and was not run.
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

- Append-only stream families, with `run:*` carrying typed run events (no mutation of past records).
- Per-append atomicity in stream stores: each append is all-or-nothing.
- Content addressing for manifests, snapshots, facts, and outputs.
- Canonical JSON for hashing structured data (target semantics: RFC 8785 / JCS-style).
  - Hashed structures MUST NOT contain floats (use integer-scaled values or decimal strings).
- No ambient IO in state logic: route network/FS through an IO abstraction that supports live and replay.
- Secrets must not appear in persisted surfaces:
  - manifests, events, artifacts (including fact payloads and context snapshots), CLI/API outputs, or error details.

## Verification Workflow

The exact build lanes, gate composition, artifact policy, and selection rules live in
`docs/build-and-verification.md`. `nixfied.nix` is authoritative for the current task graph.

Use the incremental development lane for the inner loop:

```bash
nix develop
cargo fmt --all -- --check
cargo check -p <package>
cargo test -p <package> [test-filter]
```

Run the smallest relevant test target when one exists. Add affected dependent packages when a
public crate contract changes. Use `nix run .#quick` when workspace-wide library/binary checking is
useful; it is an incremental feedback command, not a test or merge gate. Do not default to
`cargo test --workspace` when a package or test target covers the change.

For a one-off parity check that already has a Nixfied leaf, run only that leaf so Nixfied starts its
declared service requirements:

```bash
nix run .#run -- --task <task-id>
```

For repeated debugging, it can be faster to start only the required service once and use focused
Cargo tests with explicit variables such as `DATABASE_URL` or `MFM_RUNTIME_CONFIG_FILE`.

Select final verification from the change surface:

| Change surface | Verification before handoff |
| --- | --- |
| Documentation or comments only | Check the changed links, examples, and command claims; run `git diff --check`. No Rust build is required unless the documentation changes an executable/generated contract or makes claims that need validation against one. |
| Local implementation in one crate | Run rustfmt, a package-scoped check or Clippy invocation, and the affected package/test targets. Test dependents when a public contract changed. |
| Cargo manifest, workspace metadata, Cargo-enforced crate taxonomy, or dependency-boundary configuration | Run affected package checks/tests and `nix run .#run -- --task cargo-metadata-contract`. Add `.#check` when workspace resolution or all-feature lint coverage changed broadly. |
| Cross-crate public API, proc-macro output, shared kernel/runtime semantics, or multi-crate behavior | Run `nix run .#check` and `nix run .#test`, unless the final `.#ci` run below will cover them. |
| PostgreSQL migration, SQLx metadata/query, Postgres store behavior, or DB gate change | Run focused tests while iterating, then `nix run .#test-db`. Add `.#check` or `.#test` only when their surfaces also changed. |
| Nixfied model or verification task graph | Run `nix run .#model-check` early, exercise the changed task/gate, and run `nix run .#ci` once on the final revision. Follow `docs/UPGRADE.md` for a Nixfied pin/runtime ABI change. |
| Flake output, package/dev-shell definition, flake dependency pin, or CI workflow | Run `nix flake check --no-build` for early evaluation, then exercise the affected output or invocation. Add `.#ci` when the toolchain, Nixfied runtime, gate execution, or cross-platform behavior changed. |
| Security-sensitive, persisted-contract, scheduler/recovery, cross-cutting, release, or explicit full local merge-readiness validation | Run focused checks first, then `nix run .#ci` once on the final revision. |

When a change spans rows, combine only the non-overlapping coverage. Editing an architecture or
workflow document does not by itself select the code/configuration row with the same subject.

`nix run .#ci` already composes `.#check`, `.#test`, and `.#test-db`, then adds the remaining parity
coverage and closing Git revision evidence. Do not run those three component gates immediately
before `.#ci` on an unchanged tree. Use them independently when that is the smallest sufficient
gate or when diagnosing a failure.

The `closing-source-revision` evidence records `HEAD`, not uncommitted changes. Treat it as final
closure evidence only when the tested worktree is clean; otherwise report that limitation.

Broad gates use `target/verification`; focused Cargo and `.#quick` use the ordinary incremental
`target`. Do not routinely clean either target. Use
`cargo clean --target-dir target/verification` only for a deliberate cold run or suspected Cargo
artifact corruption.

## Key Docs

- `README.md`: project disclaimer.
- `docs/code-quality.md`: mandatory quality policy for all changes.
- `docs/architecture.md`: taxonomy, placement rules, and architecture boundaries.
- `docs/build-and-verification.md`: Rust artifact ownership, build lanes, and gate contract.
- `docs/design.md`: full design contract (authoritative).
- `bin/cli/README.md`: CLI behavior and JSON output contract.
- `crates/kernel/runtime/README.md`: typed runtime scheduler and recovery concepts.
- `crates/kernel/program/README.md`: typed program authoring concepts.
- `crates/kernel/program-derive/README.md`: proc-macro notes.

## Key Design Principles

- Modularity: each crate should be usable as a library with minimal coupling.
- Type Safety: strong typing throughout with minimal use of dynamic dispatch
- Explicit boundaries: keep public APIs stable; avoid leaking implementation details.
- Performance as a feature: avoid accidental allocations in hot paths; measure first.
- Extensibility: prefer traits + generic types when it improves composability.
- Correctness and security first: especially in security related modules (e.g. `mfm_core::keystore`).
- Output stability: treat CLI JSON/text formats as public API.

## Nixfied Customization

Nixfied v2 is a flake input. Keep project task/composite/service changes in `nixfied.nix`; do not
recreate v1 framework, dispatcher, or introspection trees. `flake.nix` owns the generated app and
package exposure, and `flake.lock` owns exact upstream pins. See `docs/build-and-verification.md`
for build-lane details and `docs/UPGRADE.md` for coordinated pin/runtime changes.

## Code Style & API Guidelines

### Rust Style

- Prefer explicit, readable code over cleverness.
- Avoid panics in library code. Use `Result` and typed errors.
- Keep public APIs documented and consistent (names, error behavior, invariants).
- Avoid cloning in hot paths. Prefer `&str`/borrowing where possible.

### Error Handling

- Libraries:
  - Prefer typed error enums/structs for stable, testable behavior.
  - Use `thiserror` for structured error enums/structs when it can preserve the
    intended public message, source chain, and redaction boundary.
  - Use reusable `impl From<LowerLevelError> for DomainError` conversions as the
    standard propagation pattern. Prefer `#[from]` on `thiserror` variants when
    the conversion is a direct source-preserving wrapper, and prefer manual
    `From` impls when the mapping classifies, redacts, or sanitizes lower-level
    diagnostics.
  - Keep conversion boundaries intentional: retain source errors in variants when
    callers need to inspect them, and convert to public-safe strings/codes at
    CLI/API or trust boundaries.
  - Use `anyhow` only for executable/glue code where the error is not part of a
    library contract.
- CLI:
  - Preserve stable, machine-readable error codes (see `bin/cli/README.md`).
  - Avoid breaking the JSON output schema.

### Op vs State Placement Contract

- `Operation` (`expand`): planning-only (`config -> graph`), deterministic, no ambient IO.
    - If code builds typed state graph nodes or dependency edges, place it in an op crate.
- `State` (`handle`): execution-only (runtime behavior through context, explicit capabilities, and recorder).
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
- Be careful with format versioning: keystore files are persisted JSON. If you add fields, bump the format deliberately or fail closed.
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

### Typed Runtime (`crates/kernel/runtime/`)

The typed runtime is async, event-sourced, and drives certified specs through the FSM scheduler.

- Keep scheduler/recovery logic deterministic unless it delegates to an explicit side-effect runner.
- Do not block the async runtime:
  - use async I/O where possible
  - use `tokio::task::spawn_blocking` for CPU-bound or blocking work
- If you change scheduling/recovery semantics, add tests that assert behavior.

### Proc-Macros (`crates/kernel/program-derive/`)

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
