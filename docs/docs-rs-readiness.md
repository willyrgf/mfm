# docs.rs Publishing Readiness Guide

Generated: 2026-05-25

Purpose: checklist for keeping public crate documentation aligned with the typed-core architecture
and the `publish-docs` catalog.

## Current State

The active documentation wave is typed-core first. Removed dynamic crates are not publish targets
and should not appear in publish waves, crate examples, or umbrella docs.

Canonical package metadata lives in:

- `crates/docs/catalog.toml`
- `crates/docs/publish-wave.json`
- `crates/docs/README.md`

Use `nix run .#publish-docs -- plan` for release planning and `nix run .#publish-docs --
sync-umbrella --check` to verify umbrella README freshness.

## Current Wave

The first docs.rs wave contains:

| Group | Packages |
|---|---|
| typed kernel | `mfm-ids`, `mfm-canonical`, `mfm-values`, `mfm-effects`, `mfm-capabilities`, `mfm-program`, `mfm-program-derive`, `mfm-spec`, `mfm-certify`, `mfm-events`, `mfm-store`, `mfm-replay` |
| foundation | `mfm_core`, `mfm-evm-core` |
| umbrella | `mfm-docs` |

`mfm-runtime` is cataloged as a typed-core public package, but the current first wave keeps runtime
publication after the lower-level contracts it consumes.

## Cataloged Typed Packages

| Section | Packages |
|---|---|
| core | typed kernel crates, `mfm-authored-config`, `mfm_core`, `mfm-evm-core`, portfolio model/config |
| states | `mfm-state-portfolio` |
| ops | proof and portfolio tracker typed planners |
| transports | proof, portfolio, and process execution typed backends |
| storages | typed Postgres run-event store and filesystem artifact store |
| tools/docs | `mfm-publish-docs`, `mfm-docs` |

## Global Requirements

These apply to every public crate:

1. Preserve `#![warn(missing_docs)]` where it is enabled.
2. Every crate root must explain its typed-core responsibility and owner boundary.
3. Public items and public fields need rustdoc when the crate warns on missing docs.
4. Examples must use active typed APIs and active package names.
5. Examples must not introduce floats into hashed typed values/configs.
6. Examples must not include secrets, private keys, passwords, mnemonics, or live credentials.
7. Runtime, replay, store, and transport docs must distinguish live execution from replay.
8. Storage docs must describe typed event/artifact authority, not removed dynamic store traits.

## Priority Tiers

### Tier 1: Typed Kernel Contracts

Prioritize:

- identity grammar and versioning examples
- canonical JSON and digest examples
- derive examples for typed values/configs/public outputs
- state-program authoring examples
- certification failure examples
- event and store commit invariants
- replay broker and verifier examples

### Tier 2: Runtime, Replay, Store, And Test Support

Prioritize:

- certified spec start/resume/replay flow examples
- projection rebuild and side-effect ledger documentation
- replay evidence examples with no live capabilities
- typed certified slice fixture documentation

### Tier 3: Domain Model And Config Crates

Prioritize:

- canonical JSON/TOML authoring examples
- no-float/no-secret value guidance
- EVM typed JSON wrapper examples
- portfolio stable domain-key examples
- keystore threat model and redaction notes in `mfm_core`

### Tier 4: States, Ops, And Transports

Prioritize:

- each state crate's typed state contract table
- operation planner examples that produce certified specs
- runner backend docs that explain live versus replay behavior
- side-effect idempotency and receipt/recovery evidence docs

### Tier 5: Storage And Tooling

Prioritize:

- typed Postgres event-store setup
- typed filesystem artifact-store setup
- `publish-docs` plan/apply/resume examples

## Validation Workflow

For documentation-only changes:

```bash
nix run .#check
```

For crate-level rustdoc work:

```bash
cargo doc --no-deps -p <package>
cargo test -p <package> --doc
```

For publish planning:

```bash
nix run .#publish-docs -- plan
nix run .#publish-docs -- sync-umbrella --check
```

Use real publishing commands only from an intentional release context with registry credentials
configured.

## Style Guidelines

- Prefer small examples that compile without external services.
- Link to `docs/design.md` for runtime semantics and `docs/architecture.md` for crate placement.
- Keep docs.rs package docs focused on the crate's boundary, not the whole system.
- Mention replay/live differences when documenting transports or side-effect states.
- Document security-sensitive behavior with explicit redaction and persistence rules.
- Avoid references to removed crates, removed store traits, or removed workflow APIs except in
  negative tests that prove they are not active surfaces.
