# mfm-store

Typed kernel crate for certified run event commit contracts.

`docs/design.md` is the normative typed-core authority contract.
This crate is framework-owned and must remain domain-free. It must not depend on old dynamic
machine or SDK crates.

Production run mutation is `append_prepared_typed_commit(PreparedTypedCommit)`. A prepared commit
contains the event payload batch and artifact evidence that must become run authority atomically
with that batch. Store implementations own sequence, ordinal, event id, logical-key, precondition,
artifact evidence, and projection validation.

Synthetic direct mutation helpers are non-execution tooling only. They may be used by explicitly
named storage contract, corruption, migration, or repair fixtures, but app, CLI, REST, transport,
runtime scheduling, replay, public-output, and positive conformance paths must enter through
prepared typed commits.
