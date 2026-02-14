# Enforce Thin Binary Layers (CLI/REST)

## Why

`REDESIGN.md` requires:

- Binaries (`bin/cli`, `bin/rest-api`) are thin wrappers.
- Business execution must happen inside state-machine runs.
- Binaries should start/resume runs and render stable outputs only.

Current tx signing/sending behavior implemented directly in CLI support modules violates that boundary.

## Goal

Refactor keystore tx execution so:

- CLI keeps current UX and output schema.
- CLI does not contain tx signing/send domain logic.
- Shared ops/state-machine layers own the implementation.
- REST remains a generic start/resume run surface.
- Documentation and agent guidance continuously enforce this boundary in future cycles.

## Locked Decisions

- Keep `keystore tx-sign` and `keystore tx-send-raw` commands (backward-compatible surface).
- Move execution logic to a new ops crate: `crates/ops/keystore-tx-op`.
- Use `crates/ops/common` only for reusable primitives, not full workflow orchestration.
- Do not add dedicated REST endpoints for this flow; use generic run APIs.

## Target Architecture

### Binary responsibilities

- Parse input.
- Build op input payloads.
- Execute ops through SDK execution endpoints (no command-local run bootstrap internals).
- Decode final report via SDK helper from final snapshot context.
- Render text/json output.

### Ops responsibilities

- Expand op config into state graph.
- Perform keystore key resolution, EIP-1559 signing, secure raw-tx file output.
- Perform raw tx submission via RPC.
- Emit domain events and final report payload.

### Shared reusable layer (`crates/ops/common`)

- EIP-1559 payload builder/encoding/sign helper.
- Recovery id derivation helper.
- Raw tx hex validation and secure file I/O helpers.
- JSON-RPC submission/error normalization helper.

## Public Interface Contracts (Must Stay Stable)

### `mfm_cli keystore tx-sign`

Output fields:

- `from`
- `to`
- `nonce`
- `chain_id`
- `tx_type`
- `payload_hash`
- `out_path`

Never output:

- private key
- signature bytes
- raw tx payload

### `mfm_cli keystore tx-send-raw`

Output fields:

- `tx_hash`
- `rpc_url_host`
- `submitted_at`

Never output:

- raw tx payload

## Implementation Plan

1. Create `crates/ops/keystore-tx-op` and register it in workspace.
2. Define op IDs/versions:
   - `keystore_tx_sign` (`v1`)
   - `keystore_tx_send_raw` (`v1`)
3. Move signing/RPC/file logic from CLI support modules into ops/common + op states.
4. Wire ops into shared registry/catalog used by CLI and REST.
5. Refactor CLI commands to wrapper-only behavior:
   - map args -> op input
   - call SDK single-op execution/report helper
   - render existing output schema
6. Remove or reduce CLI helper modules to non-domain adapter role only.
7. Update docs to reflect wrapper model and op-based execution:
   - `ARCHITECTURE.md` (thin-wrapper boundary reminder)
   - `bin/cli/README.md` (commands are wrappers over run execution)
   - `bin/rest-api/README.md` (generic run surface, no business logic in handlers)
   - this file (`ENFORCE_BINS_THIN_LAYER.md`) if policy/details evolve
8. Update `AGENTS.md` with explicit enforcement rules and contributor checklist items.

## Documentation and Governance Updates (Required)

To keep this boundary intact over time, every change that adds or modifies CLI/REST behavior must:

- Confirm business execution lives in `crates/ops/*` (and reusable pieces in `crates/ops/common`).
- Keep binaries limited to parse/launch/resume/render behavior.
- Keep single-op report extraction logic in reusable crate code (`crates/sdk`/`crates/app`), not command modules.
- Update docs when interfaces or boundaries change:
  - `bin/cli/README.md`
  - `bin/rest-api/README.md`
  - `ARCHITECTURE.md` (if boundary semantics change)
  - `AGENTS.md` (if contributor rules/checklists change)

## PR Review Checklist (Thin Layer Enforcement)

For any binary feature PR, reviewers should verify:

- No domain workflow logic was added under `bin/cli` or `bin/rest-api`.
- Binary code only maps transport input/output to run start/resume/report.
- New or changed business behavior is covered by ops-level tests (not only CLI handler tests).
- CLI/API payload schema stability is preserved and documented.
- Secret-handling invariants remain preserved across outputs, errors, events, and artifacts.

## Tests and Validation

### Unit

- EIP-1559 encoding/signing determinism.
- Recovery id derivation.
- Raw tx validation/file permission behavior.
- RPC error mapping stability.

### Integration

- Keep parity test `bin/cli/tests/parity_keystore_reth_tx_send.rs` and ensure it exercises wrapper->op path.
- Add at least one op-level run test (non-CLI) for direct launcher execution.

### CI

- Keep parity step `parity-keystore-reth-tx-sign-send`.
- Full parity must pass after refactor:
  - `nix run .#ci -- --parity --summary`

## Acceptance Criteria

- No tx signing/send domain logic in `bin/cli` or `bin/rest-api`.
- CLI command names/flags/output schema remain backward-compatible.
- Implementation is reusable via shared ops layer.
- Secrets are not leaked in outputs/errors/events/artifacts.
- Parity and quality checks pass.
