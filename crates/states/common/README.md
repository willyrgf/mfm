# mfm-op-common

Reusable state primitives and helpers shared by operation crates.

This crate is intentionally ops-layer only:
- it depends on `mfm-machine` contracts
- it does not modify runtime/planner semantics
- it provides small, explicit building blocks for `states -> operations -> pipelines`

Core modules:
- `errors`: stable SDK/state error constructors and IO error mapping
- `ctx`: context read/write helpers with stable error mapping
- `states`: reusable state metadata and EVM read states
- `output`: shared output artifact persistence + event emission helper
- `idempotency`: helper builders for idempotency key shapes
- `rpc`: JSON-RPC response assertion helpers
- `test_support`: shared test harness helpers for ops crates
