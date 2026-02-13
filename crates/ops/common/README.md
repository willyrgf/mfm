# mfm-op-common

Reusable state primitives and helpers shared by operation crates.

This crate is intentionally ops-layer only:
- it depends on `mfm-machine` contracts
- it does not modify runtime/planner semantics
- it provides small, explicit building blocks for `states -> operations -> pipelines`
