# mfm-store

Typed kernel crate for certified run event commit contracts.

`docs/design.md` is the normative typed-core authority contract.
This crate is framework-owned and must remain domain-free.

Production run mutation is `append_prepared_commit_bundle(PreparedCommitBundle)`. Bundles contain a
purpose-specific `PreparedCommit<Purpose>` plan plus the artifact bytes or explicit existing
artifact admissions that must become run authority atomically with the event batch. Store
implementations own sequence, ordinal, event id, logical-key, precondition, artifact bytes,
artifact evidence, and projection validation.

`mfm-store` also owns the one retained-artifact read contract. It derives exact
`EventArtifactRequirement` values from event reference facts, exposes
`RetainedArtifactReadProvider`, and returns `VerifiedRetainedArtifactBytes` only after checking the
bytes and every typed evidence and producer binding field.

Synthetic direct mutation helpers are non-execution tooling only. They may be used by explicitly
named storage contract, corruption, migration, or repair fixtures, but app, CLI, REST, transport,
runtime scheduling, replay, public-output, and positive conformance paths must enter through
prepared typed commits.
