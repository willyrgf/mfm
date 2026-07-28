# mfm-store

`mfm-store` owns the recoverability-v1 run-journal boundary.

Its public surface has one current model:

- `CommittedRunJournal` is the immutable physical journal and exact retained-object closure
  accepted by the store verifier.
- `VerifiedRunView` is the opaque callback-free semantic fold used by drive, replay, export, and
  the store's private public/audit/trace projection paths.
- `VerifiedPublicRunView` is the owned annex-validated status and certified-public-output response.
  It grants no journal or object authority.
- `AdmitRun`, `CommitTransition`, `AuthorizeExternalAccess`, and
  `ObserveExternalAccess` are the only prepared append variants.
- `RunJournalStore` is the purpose-authorized application surface.
- `RunJournalBackend` is the durable adapter seam. It receives store-created verifiers and has no
  raw append operation.

Backends must publish the complete assigned commit, its object admissions and bindings, and its
tenant fact coordinate atomically. They revalidate every non-admission successor against the
current `VerifiedRunView` inside the same serialization boundary before assignment.

Only `Drive`, `Replay`, and `Export` authorities satisfy the sealed
`CommittedJournalLoadGrant` bound for a complete journal load. `ReadPublic`, `InspectAudit`, and
`InspectTrace` authorities use dedicated store methods instead. A public read performs one backend
load, verifies the complete history, projects the reviewed active fields and certified output
subtrees, and returns only `VerifiedPublicRunView`. Audit returns a bounded head-fixed
`VerifiedAccessAuditPage`. Trace uses a two-phase source-requirements token and returns a bounded
`VerifiedTransitionTracePage` after each disclosed source has its own inspection decision.

Reserved prior-run fact selection uses a fresh affine permit to scan the authoritative writer's
dense publication prefix through its frozen tenant barrier. Each final selected publication is
reduced to an exact publication-prefix typed closure before the completed scan can leave the
private builder. `ContentRef` transport authorities are retained as graph material but excluded
from semantic object references; typed `ValueRef` authorities remain semantic and recursively
close their payload and evidence dependencies. A `ContentRef` target whose schema is exactly
`mfm.value-ref.v1` remains a transport-only wrapper and stops traversal because the incoming
content reference is already the semantic edge.

The generic observation append atomically persists the returned response, its source-closure
attestation, all exact object bindings, and one private attestation-routing row. Live reducer entry
after compatible evidence selection and replay completeness load that row against an exact verified
view and share the backend completeness predicate. `CompletedFactScan` consumes itself into that
generic material and derives its sealed authorization reference internally.

`RunAccessAuthority<G>` values are bound to one exact store instance and purpose. The paired
`RunAccessAuthorityIssuer` is non-cloneable. Only a directly observed newly appended external
authorization returns `NewlyAppendedAuthorization`; idempotent, stale, rejected, and ambiguous
outcomes never recreate live-operation authority.

Portable replay may call `verify_offline_recorded_history` with decoded immutable rows and their
complete exact object closure. That function runs the same physical verifier and semantic reducer
as an authorized backend load and creates no live store or mutation authority.

With the `test-support` feature, `AsyncInMemoryRunStore::new(StoreIdentity)` returns the store and
its sole issuer as a pair. The in-memory backend stages changes in a scratch copy and swaps once,
preserving the same all-or-nothing and compare-and-swap contract required of durable backends.

[`docs/design.md`](../../../docs/design.md) is the authoritative semantic contract.
