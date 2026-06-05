# docs.rs Readiness Guide

Generated: 2026-06-05

Purpose: checklist for keeping public crate documentation aligned with the typed-core
architecture while crate publication remains manual.

## Current State

Public crate documentation is intentionally maintained without an automated release planner until
the API surface stabilizes. Removed workflow-shaped crates are not publish targets and should not
appear in crate examples, umbrella docs, or release notes.

Canonical package navigation lives in:

- `crates/docs/README.md`
- `docs/repo-map.md`
- `docs/repo-index.json`

## Current Wave

The first docs.rs wave should prioritize:

| Group | Packages |
|---|---|
| typed kernel | `mfm-ids`, `mfm-canonical`, `mfm-values`, `mfm-effects`, `mfm-capabilities`, `mfm-program`, `mfm-program-derive`, `mfm-spec`, `mfm-certify`, `mfm-events`, `mfm-store`, `mfm-replay` |
| foundation | `mfm_core`, `mfm-evm-core` |
| platform contracts | `mfm-artifact-capabilities`, `mfm-evm-capabilities`, `mfm-signing`, `mfm-evm-signing`, `mfm-adapter-contracts` |
| umbrella | `mfm-docs` |

`mfm-runtime` remains a typed-core public package, but it should publish after the lower-level
contracts it consumes.

## Public Package Groups

| Section | Packages |
|---|---|
| core | typed kernel crates, capability contracts, `mfm-authored-config`, `mfm_core`, EVM core/contract model/config, portfolio model/config |
| states | `mfm-state-evm-contracts`, `mfm-state-portfolio` |
| ops | EVM contract lifecycle, proof, and portfolio tracker typed planners |
| adapters | EVM contract lifecycle and portfolio capability bindings |
| signers | generic signing, EVM signing, and keystore signer provider contracts |
| transports | generic EVM JSON-RPC, proof, and process execution typed backends |
| storages | typed Postgres run-event store and filesystem artifact store |
| docs | `mfm-docs` |

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

### Tier 4: States, Ops, Adapters, And Transports

Prioritize:

- each state crate's typed state contract table
- operation planner examples that produce certified specs
- adapter docs that show capability binding without live transport ownership
- transport docs that explain live versus replay behavior
- side-effect idempotency and receipt/recovery evidence docs

### Tier 5: Storage And Docs

Prioritize:

- typed Postgres event-store setup
- typed filesystem artifact-store setup
- manual `mfm-docs` README updates for public package navigation

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
