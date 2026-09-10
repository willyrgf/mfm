# MFM Development Guide for AI Agents

This file is the repository source of truth for agent behavior. `docs/build-and-verification.md`
owns verification selection and `nixfied.nix` owns the executable task graph.

## Non-negotiables

- Divide non-trivial work into ordered logical commits. Each commit must leave one coherent current
  design; keep inseparable cutovers together.
- Follow `docs/code-quality.md` for code, tests, documentation, build, and workflow changes.
- Treat reducing complexity as an objective of every non-trivial change. Prefer the smallest
  coherent design that satisfies current requirements. Minimize concepts, public APIs,
  configuration points, code paths, duplicated responsibilities, and future change sites.
- Before adding an abstraction or extending existing machinery, identify what can instead be
  simplified, unified, or deleted within the affected scope. Do not preserve unnecessary complexity
  merely because it already exists. This does not authorize unrelated rewrites.
- Before adding a layer, ask whether an existing layer can be removed. Before adding a special
  case, ask whether the representation is wrong.
- Delete superseded implementations, APIs, tests, and documentation in the same cutover. Preserve
  coverage of retained behavior. Do not add wrappers that leave the old complexity underneath.
- Prefer fewer LOC when behavior and guarantees are equivalent. Do not reduce LOC through
  compressed code, weaker validation, omitted tests, or blurred ownership boundaries.
- For non-trivial changes, report what was simplified and removed, what necessary complexity was
  added, and the resulting production-code LOC change. Explain increases; use LOC as evidence,
  not a quota, and do not optimize the metric at the expense of the design.
- Verification is scope-driven. Use the narrowest command that exercises changed behavior, then
  expand only when the affected boundary or risk requires it.
- Commit subjects are lower case.
- Never log, print, or persist passwords, mnemonics, private keys, or credentials.
- Preserve crate boundaries and keep libraries usable without the CLI.
- Add dependencies only with strong justification.
- Do not commit third-party vendored source or use external git/path patches without explicit user
  approval.
- Treat keystore and cryptographic changes as high risk and strengthen tests explicitly.

## When architecture is unclear

Read `docs/design.md` and `docs/architecture.md`. If architecture, ownership, or a design contract
is still unclear, stop implementation and ask one dedicated architect agent for a single target
design, complete cutover/deletion scope, affected contracts/tests, and a logical commit sequence.
The design should minimize concepts, code paths, public types, duplicated responsibilities, future
change sites, and LOC; breaking changes and physical deletion are allowed.

Every planning, architecture, and design response includes a `Material uncertainties` section.
For each material uncertainty, state the assumption, why it is uncertain, the consequence if wrong,
and how to validate it. Write `none` when none remain. Architecture/ownership uncertainties trigger
the architect rule above.

## Architecture and design invariants

- `docs/design.md` is authoritative. Fix code that disagrees, or update the contract and its tests in
  the same change.
- `docs/architecture.md` owns taxonomy and placement:
  - Program is an immutable, content-addressed linear State sequence;
  - Runtime associates the Program with typed implementations and owns the sole semantic fold;
  - State implementations are deterministic and perform no ambient IO;
  - Read adapters bind typed intent to explicit observational capabilities;
  - Journal owns the exact append-only frame wire and history qualification;
  - Store owns mechanical complete-prefix load and atomic exact-head append only;
  - transports and signers remain reusable platform primitives; and
  - binaries parse/render their supported transport surface only.
- Run histories are append-only; every append is all-or-nothing and prior frames never mutate.
- Programs, values, frames, and outputs are content addressed.
- Structured hashing uses canonical JSON with JCS-style semantics. Hashed structures contain no
  floats; use scaled integers or decimal strings.
- Network and filesystem IO cross explicit adapter or Store abstractions.
- Secrets never appear in Program, admitted context, Journal, Store metadata, RunView, public output,
  or error detail.
- One explicit caller-supplied RunId identifies a run. Writable rollback of acknowledged history is
  unsupported.

## Pinned development and verification

All direct Cargo/Rust tooling runs in the default Nix development shell. Never rely on host Rust.
Use `docs/build-and-verification.md` for commands and selection. `nixfied.nix` owns task IDs and gate
composition. Follow the pinned Nixfied adopter guide for framework changes.

- Do not run broad gates merely because a commit is about to be created.
- Do not run `.#check`, `.#test`, and `.#test-db` immediately before `.#ci`; CI composes them.
- Run one final `nix run .#ci` on the exact candidate when the selected workflow requires it.

## Rust and API rules

- Prefer explicit readable code, borrowing, async IO, and typed errors. Avoid panics in libraries,
  accidental hot-path allocations, and `unsafe`.
- Put unavoidable blocking pure work in `spawn_blocking`, await it immediately, and never move IO or
  mutation authority into the blocking closure.
- Keep one current documented public API and `#![warn(missing_docs)]` in libraries.
- Prefer `thiserror` with source-preserving conversions. Reserve `anyhow` for executable glue.
- Convert failures to reviewed redaction-safe codes/messages at CLI/API boundaries.
- Comments explain constraints and invariants, not obvious code or the current PR.
- Do not make `Keystore` `Send` or `Sync` without a deliberate redesign.

## Surface-specific rules

- Program/Runtime: keep sequence association and recovery scheduling deterministic; test hot/cold equivalence,
  cancellation safety, typed error mapping, and semantic changes.
- Journal/Store: keep exact wire qualification in Journal and physical atomicity in Store. Store must
  not acquire Program, domain, capability, or reducer semantics.
- PostgreSQL: preserve one repeatable-read complete load snapshot and one advisory-locked,
  synchronous-commit append transaction. Ambiguous COMMIT acknowledgement is `Indeterminate`.
- Adapters: only duplicate-safe Reads are supported. Local mismatch is `Internal` with no provider
  call or append; only authenticated external evidence can durably represent an integrity block.
- CLI/REST: read the relevant binary README and retain its one current redacted transport contract.
- Proc macros: keep expansion small and diagnostics clear; test in a consuming crate.

## Tests and documentation

- Prefer boundary-focused unit, regression, compile-fail, and integration tests. Add a regression
  test when a bug can recur.
- Keep large implementations separate from large test modules.
- Update `docs/architecture.md` for responsibility changes and `docs/design.md` for Program,
  Runtime, Journal, Store, or persistence-contract changes.
- Update transport docs with schema/behavior changes and rustdoc examples when entry points change.
- Documentation is part of the change, not follow-up work.
