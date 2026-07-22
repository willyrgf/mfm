# MFM Development Guide for AI Agents

This document is the source of truth for AI-agent behavior and contribution rules in this
repository. `docs/build-and-verification.md` owns workflow mechanics and verification selection;
`nixfied.nix` owns the executable task graph.

## Non-Negotiables

- Divide non-trivial work into ordered logical commits. Each commit must leave one coherent current
  design; keep inseparable cutovers together instead of staging compatibility paths.
- Follow `docs/code-quality.md` for every code, test, documentation, build, and workflow change.
- Verification is scope-driven, not commit-driven. Use the narrowest command that exercises the
  changed behavior, and expand only when the affected boundary or risk requires it.
- Write commit subjects in lower case, for example `fix nix task wrappers to preserve caller cwd`.
- Never log, print, or persist secrets (passwords, mnemonics, private keys).
- Preserve crate boundaries and keep libraries usable without the CLI.
- Add dependencies only with strong justification.
- If you touch security-sensitive code (keystore/crypto), be explicit and add or strengthen tests.

## When Architecture Is Unclear

Before implementation, spawn a dedicated architect agent when architecture, ownership, or
design-contract direction remains unclear after reading `docs/design.md` and
`docs/architecture.md`. Pass it the relevant context and these requirements explicitly: minimize
concepts, code paths, public types, duplicated responsibilities, future change sites, and LOC; allow
breaking changes; delete superseded code without compatibility paths or fallbacks; and divide the
work into logical commits.

Ask the architect agent for one target design, the complete cutover and deletion scope, affected
contracts and tests, and a logical commit sequence. Resolve the ambiguity before adding code; do not
use parallel implementations as a substitute for a decision.

Every planning, architecture, and design response must include a `Material uncertainties` section
before implementation begins. List every material choice or assumption for which confidence is not
high. For each, state the choice or assumption, why it is uncertain, the consequence if it is wrong,
and how to resolve or validate it. Write `none` when no material uncertainty remains. Do not pad the
section with routine, low-impact choices. Any item involving architecture, ownership, or the design
contract triggers the architect-agent rule above.

## Architecture and Design Invariants

- `docs/design.md` is authoritative. If code disagrees with it, fix the code or deliberately update
  the contract and its tests in the same change.
- `docs/architecture.md` owns taxonomy and placement:
  - operations deterministically assemble state graphs and perform no ambient IO
  - states own reusable domain semantics
  - adapters bind state intent to explicit capabilities
  - transports and signers remain reusable platform primitives
  - binaries parse input, start/resume runs, and render outputs only

Key invariants to preserve (high risk if violated):

- Append-only stream families, with `run:*` carrying typed run events (no mutation of past records).
- Per-append atomicity in stream stores: each append is all-or-nothing.
- Content addressing for manifests, snapshots, facts, and outputs.
- Canonical JSON for hashing structured data (target semantics: RFC 8785 / JCS-style).
  - Hashed structures MUST NOT contain floats (use integer-scaled values or decimal strings).
- No ambient IO in state logic: route network/FS through an IO abstraction that supports live and replay.
- Secrets must not appear in persisted surfaces:
  - manifests, events, artifacts (including fact payloads and context snapshots), CLI/API outputs, or error details.

## Pinned Development and Verification

All direct Cargo/Rust tooling must run in the default Nix development shell; never rely on
host-installed Rust tools. [`docs/build-and-verification.md`](docs/build-and-verification.md) owns
the commands, verification-selection matrix, build lanes, gate composition, internal-task caveats,
and artifact lifecycle. [`nixfied.nix`](nixfied.nix) owns the exact executable graph and task ids.

Follow the upstream
[Nixfied adopter guide](https://github.com/willyrgf/nixfied/blob/HEAD/docs/GUIDE.md) for framework
integration and coordinated pin/runtime changes. Two rules are non-negotiable:

- do not run broad gates merely because a commit is about to be created
- do not run `.#check`, `.#test`, and `.#test-db` immediately before `.#ci` on the same tree;
  `.#ci` already composes them

## Rust and API Rules

- Prefer explicit, readable code; avoid panics in libraries and accidental allocations in hot paths.
- Prefer borrowing over cloning, async IO over blocking, and `spawn_blocking` for unavoidable blocking work.
- Keep the one current public API documented and internally consistent. Keep
  `#![warn(missing_docs)]` enabled in libraries.
- Use typed library errors. Prefer `thiserror` and source-preserving `From` conversions; use manual
  conversions when classifying or redacting diagnostics. Reserve `anyhow` for executable glue.
- Convert errors to reviewed, redaction-safe public codes/messages at CLI/API boundaries.
- Comments explain constraints and invariants, not the obvious code or the current PR.
- Avoid `unsafe`. If unavoidable, document its safety invariant and add tests that exercise it.

## Surface-Specific Rules

- Keystore/crypto: treat as high risk; preserve bounded-input validation, AAD anti-swap binding,
  zeroization, and constant-time comparisons. Never expose secrets, fail closed on persisted-format
  changes, and add tamper/corruption tests. Do not make `Keystore` `Send`/`Sync` without a deliberate
  redesign.
- CLI/REST: read the relevant binary README, emit its one current text/JSON and redacted-error
  contract, support non-interactive use, and keep binaries as transport-only wrappers over run
  execution.
- Typed runtime: keep scheduling/recovery deterministic outside explicit side-effect runners, do not
  block the async runtime, and test semantic changes.
- Proc macros: keep expansion small and unsurprising, optimize diagnostics, and test behavior in a
  consuming crate.

## Tests and Documentation

- Prefer boundary-focused unit, regression, compile-fail, and integration tests. If a bug can recur,
  add a regression test.
- Avoid combining large implementations with large inline test modules; keep only small local smoke
  tests inline.
- Update `docs/architecture.md` for responsibility changes and `docs/design.md` for runtime,
  storage, or replay contract changes.
- Update CLI/REST documentation with transport behavior or schema changes.
- Document new public items and update/add a rustdoc example when a crate entry point changes.
- Documentation is part of the change, not follow-up work.
