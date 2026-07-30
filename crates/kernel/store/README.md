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
- `QualifiedRunStore<B>` is the affine pre-runtime assembly. It owns the only backend handle and
  mutation seal, admits qualified support, and can be split exactly once.
- `RunHistoryWriter<B>` is the non-cloneable post-bootstrap mutation capability consumed by
  `mfm-runtime`.
- `RunHistoryReader<B>` is the cloneable purpose-authorized read, replay, inspection, and export
  surface. It exposes no append or support-admission operation.
- `RunJournalBackend` is the durable adapter seam. It receives store-created verifiers and has no
  raw append operation.

Backends must publish the complete assigned commit, its object admissions and bindings, and its
tenant fact coordinate atomically. They revalidate every non-admission successor against the
current `VerifiedRunView` inside the same serialization boundary before assignment.

The writer loads only for `Drive`; the reader loads for `Replay` and `Export`.
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

`FactScanPageVerifier` alone owns the private `(fact_order, fact_ordinal)` cursor or terminal state,
the 4,096-publication and 8,192-emission step budgets, and logical emission ranges. Backends load
ordered complete publications and submit them without local continuation policy. A partial
publication is fully reloaded and verified on its next logical range; no cursor or permit is
persisted or public.

The generic observation append atomically persists the returned response, its source-closure
attestation, all exact object bindings, and one private attestation-routing row. Live reducer entry
after compatible evidence selection and replay completeness load that row against an exact verified
view and share the backend completeness predicate. `CompletedFactScan` consumes its affine permit
into immutable pending observation material, which can be reused only to persist the same logical
response and derives its sealed authorization reference internally.

The selected-source and transport graph remains unbound `RequireExisting` material, while the
response and attestation are the two bound `AdmitOrVerifyExact` products. Physical replay may
defer first-use authority only for unbound imports in an attested fact observation, because a
single-run history cannot reprove cross-run existence. The semantic fold must exact-close the
entire import set before returning a verified view; live backends still verify global exact
authority during append.

Successful transition bodies retain output and fact bindings in semantic ordinal order. Private
store assembly canonical-byte sorts the repeated output and fact binding-delta groups separately;
the fold rejects duplicate ordinals and reindexes both groups before semantic comparison and
application.

`RunAccessAuthority<G>` values are bound to one exact store instance and purpose. The paired
`RunAccessAuthorityIssuer` is non-cloneable. Only a directly observed newly appended external
authorization returns `NewlyAppendedAuthorization`; idempotent, stale, rejected, and ambiguous
outcomes never recreate live-operation authority.

Portable replay may call `verify_offline_recorded_history` with decoded immutable rows and their
complete exact object closure. That function runs the same physical verifier and semantic reducer
as an authorized backend load and creates no live store or mutation authority.

With the `test-support` feature, `open_in_memory(StoreIdentity)` returns a
`QualifiedRunStore<InMemoryRunJournalBackend>` and its sole issuer. Tests provision configured
values and admit support before consuming the assembly with `split`; the in-memory backend stages
changes in a scratch copy and swaps once, preserving the same all-or-nothing and compare-and-swap
contract required of durable backends.

[`docs/design.md`](../../../docs/design.md) is the authoritative semantic contract.
