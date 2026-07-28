# mfm-replay

Typed kernel crate for certified replay evidence brokers and verifier contracts.

`docs/design.md` is the normative typed-core authority contract.
This crate answers replay requests only from one store-owned `VerifiedRunView` and explicit
source-run evidence. It must not construct live capabilities or live transports.

The store loads committed batches, typed records, and exact retained objects under one snapshot,
validates their physical/current-format structure, and returns `CommittedRunJournal`. Physical
loading verifies no manual-resolution signatures. Binding that journal to the exact
`CertifiedTypedSpec` runs the sole store-owned semantic certified-history fold, including
deterministic `mfm-manual-auth` verification of every historical manual proof against certified
replay authority. That pass performs no live operator, signer, keystore, or policy-registry lookup
and makes no external-truth decision.

`ReplayBroker` borrows the resulting non-cloneable, fully authorization-verified
`VerifiedRunView`. Replay trusts its derived fold: it does not reconstruct or reverify historical
manual proofs, copy records, projection snapshots, artifacts, facts, side-effect ledgers, or
manual-resolution maps, and owns no duplicate historical validator. Raw status DTOs, journal JSON,
hash-only specs, rendered public output, standalone artifact bytes, or a consumer-rebuilt
projection cannot construct replay authority.

`ReplayReadAuthority<'view>` owns only explicit cross-run source-fact records and additional
`VerifiedRetainedArtifactBytes`; the primary journal, certified spec, lifecycle fold, and retained
objects remain owned by the borrowed view.

Current replay verifiers temporarily query purpose-specific borrowed readers from
`mfm_store::v1::current_lifecycle`. Those readers are a scoped migration seam, not a second replay
fold or durable API, and are deleted by the complete audited lifecycle cutover.

Long-lived replay work may refresh only by consuming its old view with a newly loaded journal
through the store's strict-successor check. Equal, truncated, divergent, reordered, or
old-object-replaced histories cannot inherit the already certified authority. The store
semantically verifies the suffix, including any new historical authorization, and replay never
merges histories or repeats that verification.

The generic pure-state verifier reconstructs certified config, arbitrary input trees, and typed
context from retained evidence, invokes the same `PureState` behavior, and compares exact canonical
output bytes. Replay does not resolve current configuration, read the current executable, mint live
execution identity, or call a provider.
