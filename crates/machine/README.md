# mfm-machine

v4 state machine runtime.

This crate will define the stable public API contract and execution engine described in `docs/design.md`.

Persisted stream records and domain events are append-boundary checked by this crate. JSON payloads
must be canonical-json-hashable, must not contain floats, and must not contain secret-shaped keys or
values. Concrete stream stores should call the machine validation helpers as defense in depth.
Resume also revalidates structured artifacts such as manifests and context snapshots: bytes must
match their content address and be the exact canonical JSON encoding of the parsed value.
Runtime start validates plan topology before `RunStarted` is persisted. Runtime resume validates
the resolved plan against historical state IDs before orphan recovery or terminal status handling.
