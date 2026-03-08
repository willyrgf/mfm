# docs.rs Publishing Readiness Guide

> Generated: 2026-03-08
> Purpose: Actionable checklist for the engineer agent adding code documentation and examples across the workspace.

## Executive Summary

**Current state: PARTIALLY READY, but not workspace-ready.**

- Many crates now enforce `#![warn(missing_docs)]`, so the earlier zero-coverage snapshot is stale.
- Several core and state crates already have crate-level docs and examples.
- The remaining gap is concentrated in item-level docs, publish-wave prioritization, and crate landing-page polish.
- This document tracks rustdoc/docs.rs readiness only; current ops/states inventory lives in `docs/ops-and-states.md`.
- Recent progress on 2026-03-08:
  - wave-1 landing pages and examples are now in place for `mfm-machine`, `mfm-machine-derive`, `mfm-machine-test-support`, `mfm-sdk`, `mfm_core::config`, and `mfm-evm-core`
  - `nix run .#check` is no longer blocked by the readonly Cargo target-dir issue in Nixfied task apps
  - publish-wave manifests now use `path + version` for the near-term docs.rs release chain

## Current Publish Order

`docs.rs` only builds crate documentation after a crate has been published to crates.io. Repository docs such as `docs/ops-and-states.md` stay in-repo; crate-level rustdoc and README content are what appear on docs.rs.

Current near-term release chain:

1. Foundation:
   - `mfm-machine`
   - `mfm-machine-derive`
   - `mfm_core`
   - `mfm-evm-core`
2. First dependents:
   - `mfm-sdk`
   - `mfm-machine-test-support`
   - `mfm-collectors-evm`
   - `mfm-collectors-exec`
   - `mfm-collectors-nix`
   - `mfm-transports-local-evm`
   - `mfm-op-keystore-shim`
3. Shared-state base:
   - `mfm-state-common`
4. Runtime layer:
   - `mfm-evm-runtime`
5. State consumers:
   - `mfm-state-keystore`
   - `mfm-state-aave-v3`

Notes:

- `cargo publish --dry-run` for a crate with `path + version` dependencies still expects the versioned upstream crate to exist on crates.io. A dry-run failure like `no matching package named 'mfm-machine' found` is expected until the earlier publish step has completed.
- Use `cargo check -p <crate> --lib` for local compile validation before the upstream versions exist in the registry.
- Use `cargo package --allow-dirty --list -p <crate>` when you want to inspect the files that would be packaged without requiring the upstream versions to exist in the registry.
- Keep `mfm-app` out of the first publish wave; its dependency surface and end-user positioning still need curation.

## Global Requirements (Apply to Every Crate)

1. Preserve `#![warn(missing_docs)]` in every `lib.rs` and upgrade targeted publish-wave crates toward stricter enforcement once coverage is complete.
2. Every `lib.rs` must have a crate-level `//!` doc block explaining purpose, relationship to the architecture, and a minimal usage example.
3. Every public item (`pub fn`, `pub struct`, `pub enum`, `pub trait`, `pub type`, `pub const`, `pub mod`) must have a `///` doc comment.
4. Every public struct/enum field must have a `///` doc comment.
5. Every trait method must have a `///` doc comment.
6. Add at least one ```` ```rust ```` doc example per crate (ideally on the main public type or entry-point function).
7. Security-sensitive items (keystore, secret store, crypto) must document threat model considerations.

## Priority Tiers

### Tier 1 — Core Engine (document first)

These crates define the execution model and are the foundation everything else depends on.

Current publish-wave framing:

- `Wave 1`: `mfm-machine`, `mfm-machine-derive`, `mfm-machine-test-support`

Remaining work in this tier:

- deepen `mfm-machine` item-level docs around IDs, event types, execution plans, and standard tags
- keep this tier as the reference foundation that downstream crate docs link back to

---

### Tier 2 — Core Primitives & SDK

Current publish-wave framing:

- `Wave 1`: `mfm_core`, `mfm-evm-core`, `mfm-sdk`
- `Later / topology-dependent`: `mfm-app`

Remaining work in this tier:

- expand item-level docs in `mfm_core` config models and security-sensitive keystore support types
- treat `mfm-app` as a later publish target because packaging topology and surface curation matter as much as raw rustdoc coverage

---

### Tier 3 — States Layer

The live catalog of current production states now lives in `docs/ops-and-states.md`.
This section tracks publish readiness, not runtime inventory.

Current publish-wave framing:

- `Wave 1`: `mfm-evm-runtime`, `mfm-state-keystore`, `mfm-state-common`
- `Wave 2`: `mfm-state-aave-v3`

Remaining work in this tier:

- improve module landing pages for state families, especially `evm-runtime` read/write modules
- add stronger security/context notes to keystore state modules
- separate test-support visibility from state-catalog visibility in `mfm-state-common`
- add crate/package metadata that improves crates.io/docs.rs landing pages

---

### Tier 4 — Ops Layer

The live catalog of registered ops now lives in `docs/ops-and-states.md`.
This section tracks planner-crate publish readiness, not the current built-in registry.

Current publish-wave framing:

- `Wave 1 within ops`: `mfm-op-keystore-admin`, `mfm-op-keystore-tx`, `mfm-op-evm-read`, `mfm-op-evm-write`
- `Wave 2`: `mfm-op-portfolio-tracker`, `mfm-op-aave-v3-origin-adapt`, `mfm-op-nix-app`, `mfm-op-evm-deploy-configure-validate`
- `Later / low priority`: `mfm-op-proof`, `mfm-op-keystore-shim`

Remaining work in this tier:

- keep planner docs focused on config validation and graph composition
- improve crate landing pages for the highest-value user-facing ops
- avoid making thin wrapper crates the primary docs.rs navigation surface
- document op-local report/output types where they remain intentional

---

### Tier 5 — Storage Layer

All 6 crates have good crate-level docs but lack item-level documentation.

| Crate | Crate Doc | Undocumented Items |
|-------|-----------|-------------------|
| `event-store-mem` | Good | `MemEventStore`, `new()` |
| `event-store-postgres` | Good | `PostgresEventStore`, `connect()`, `connect_env()` |
| `artifact-store-fs` | Good | `FsArtifactStore`, `new()` |
| `artifact-store-s3` | Good | `S3ArtifactStore`, `new()`, `from_env()` |
| `artifact-store-secret` | Good | `SecretKey` (all methods), `SecretArtifactStore` (all methods) |
| `indexer` | Good | `ProjectionIndexer`, `new()` |

---

### Tier 6 — Collectors & Transports

#### Collectors with crate-level docs (3/5):
- `collectors/evm` — Present
- `collectors/evm-jsonrpc-http` — Present (detailed)
- `collectors/nix-exec` — Present

#### Collectors MISSING crate-level docs (2/5):
- `collectors/nix`
- `collectors/exec`

#### Transports — ALL 4 MISSING crate-level docs:
- `transports/local-evm`
- `transports/local-fs`
- `transports/local-keystore`
- `transports/proof`

#### Common undocumented items:
- All struct types and fields across all 9 crates
- All constants (namespace identifiers)
- All factory constructors
- All client methods
- Zero doc examples

---

## Documentation Gaps from Prior Review

Keep only items here that directly affect docs.rs readiness:

1. **Wave-1 landing pages are in place** — remaining work is deeper item-level coverage and publish-surface curation.
2. **`crates/app/` is still a later publish target** — publishing topology is a larger issue than raw rustdoc coverage.
3. **Selected state crates are close to publishable** — preserve that status and avoid regressing `missing_docs`.

## Suggested Workflow for the Engineer Agent

1. **Finish targeted item-level docs in Tier 1** — keep improving the core engine reference surface around IDs, events, execution plans, and engine-boundary types.
2. **Finish targeted item-level docs in Tier 2** — especially `mfm_core` config types and security-sensitive support surfaces; keep `mfm-app` as a later publish target.
3. **Then Tier 3** (states) — document the shared state primitives.
4. **Then Tier 4-6** (ops, storages, collectors, transports) — these follow patterns established in earlier tiers.
5. After each crate is documented, preserve or tighten `#![warn(missing_docs)]` to prevent regressions.
6. Run `nix run .#check` after each crate to verify compilation.
7. Run `cargo doc --no-deps --document-private-items` to preview docs.rs output.

## Style Guidelines for Doc Comments

Follow existing patterns from the best-documented code (crates/core/src/keystore/):

```rust
/// Brief one-line description.
///
/// Longer explanation if needed, covering:
/// - what this does
/// - when to use it
/// - important invariants or constraints
///
/// # Examples
///
/// ```rust
/// use mfm_machine::StateId;
///
/// let id = StateId::new("my_state").unwrap();
/// assert_eq!(id.as_str(), "my_state");
/// ```
///
/// # Errors
///
/// Returns `IdValidationError` if the identifier doesn't match `^[a-z][a-z0-9_]{0,62}$`.
///
/// # Panics
///
/// (Document if applicable)
```

For security-sensitive items, include a `# Security` section:

```rust
/// # Security
///
/// This type holds secret material. The inner buffer is zeroized on drop.
/// Do not log, serialize, or expose this value in error messages.
```
