# REDESIGN Plan (M6)

This is a living checklist to align implementation with `REDESIGN.md` (v4), focusing on **semantics first**.

## Status

- [x] **Fact durability at IO-call boundary**
  - Goal: once a replayable IO call returns successfully, its `FactKey -> payload_id` binding is durable immediately
    (survives orphan attempts/crash-resume) and is persisted regardless of `EventProfile`.
  - Implemented in `crates/machine/src/live_io.rs` + `crates/machine/src/runtime.rs`.
- [x] **`RunConfig.skip_tags`**
  - Goal: executor can skip states matching configured tags (e.g. dry-run skip `apply_side_effect`) while still
    producing kernel attempt envelopes.
  - Implemented in `crates/machine/src/runtime.rs` with a regression test.

## Next Steps

- [x] **Enforce side-effect idempotency contracts**
  - `SideEffectKind::ApplySideEffect` states should require an explicit idempotency strategy (`Idempotency::Key`).
  - Enforcement: **planner validation** in `DefaultPipelinePlanner` (fail fast during plan construction).
  - Stable error code: `missing_idempotency_key`.
- [ ] **Tighten "replay correctness" invariants**
  - Audit remaining runtime paths to ensure replay mode cannot accidentally perform live IO.
  - Add targeted tests for any remaining edge cases found during audit.
