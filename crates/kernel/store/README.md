# mfm-store

Typed kernel crate for certified run event commit contracts.

`docs/design.md` is the normative typed-core authority contract.
This crate is framework-owned and must remain domain-free.

Production run mutation is `append_prepared_commit_bundle(PreparedCommitBundle)`. Bundles contain a
purpose-specific `PreparedCommit<Purpose>` plan plus the artifact bytes or explicit existing
artifact admissions that must become run authority atomically with the event batch. Store
implementations own sequence, ordinal, event id, logical-key, precondition, artifact bytes,
artifact evidence, and projection validation.

For temporary explicit fact, live-adapter, and cross-run boundaries, `mfm-store` derives exact
`EventArtifactRequirement` values from event reference facts and exposes
`RetainedArtifactReadProvider`. It returns `VerifiedRetainedArtifactBytes` only after checking the
bytes and every typed evidence and producer binding field. Per-run history does not use that
provider; the journal/view boundary below owns its retained-object authority.

Per-run reads cross one boundary: `RunJournalStore::load_committed_journal` loads committed batches,
typed records, and every required retained object under one backend snapshot. It returns an opaque,
non-cloneable `CommittedRunJournal` only after checking record identity and order, atomic batch
grouping, the store-private physical fold, and exact event-required object evidence. This physical
load does not verify manual-resolution signatures, resolve operators, consult signers or keystores,
or decide external truth.

The journal exposes its admitted spec and certificate objects only for certification bootstrap.
`CommittedRunJournal::verify` consumes the journal with the exact `CertifiedTypedSpec` and builds
the sole semantic fold in `mfm-store`. For every historical manual resolution, that fold invokes
the deterministic `mfm-manual-auth` verifier against the verifier identity, operator authority
snapshot, signing scheme, quorum, exact prefix claim, and retained proof bytes carried by certified
replay authority. It performs no live operator, signer, keystore, or policy-registry lookup and
makes no external-truth decision. Only successful semantic validation produces the opaque,
non-cloneable `VerifiedRunView`; that view is fully authorization-verified and is shared by
runtime, replay, status, and public-output reads. Those consumers trust its derived fold and do not
reconstruct or reverify historical manual proofs.

Durable adapters implement the doc-hidden `RunJournalBackend` SPI. Its load receives one
non-cloneable `JournalLoadVerifier` bound to the requested run and must consume that verifier after
loading records and objects consistently. The blanket `RunJournalStore` implementation is the
sealed permanent append/load API; there is no generic public raw-record journal minter. Store
wrappers implement `RunJournalBackend`, not `RunJournalStore`, and consume the verifier with
`accept_verified` after delegating a load. The blanket boundary rechecks the returned journal's run
id after every backend future resolves.

Neither raw record vectors, projection snapshots, standalone retained-object maps, nor independently
rebuilt consumer projections can construct that view. `current_run_sequence` is a current persisted
observation only; it is not a fabricated recoverability head token. The temporary
`current_lifecycle` borrowed readers exist only to complete the current lifecycle cutover without
copying journal authority into consumer-owned maps. They are deleted when the complete audited
lifecycle replaces the current event algebra.

A refresh consumes the old view and a newly loaded journal through
`VerifiedRunView::verify_successor`. It carries the already verified certified-spec authority
forward only when the new journal is a strict extension with the same exact committed prefix and
all previously retained objects unchanged. Equal, truncated, divergent, reordered, or
old-object-replaced candidates fail closed; a refresh does not invoke certification again.
Semantic extension applies only suffix records while resolving their exact dependencies against
the complete successor journal, including deterministic authorization verification for any new
manual-resolution record. It neither refolds the prefix nor synthesizes a suffix object map.

Synthetic direct mutation helpers are non-execution tooling only. They may be used by explicitly
named storage contract, corruption, migration, or repair fixtures, but app, CLI, REST, transport,
runtime scheduling, replay, public-output, and positive conformance paths must enter through
prepared typed commits.
