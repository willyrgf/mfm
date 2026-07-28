# mfm-replay

Callback-free recorded-history inspection and portable export for MFM.

`docs/design.md` is the normative authority contract. Every store-backed operation accepts the
exact store-owned purpose authority for replay, transition trace, access audit, or export and loads
one fresh head-bound `CommittedRunJournal`. The store consumes that journal into one opaque,
non-cloneable `VerifiedRunView`; raw records, object bytes, cursors, and prior views cannot
substitute for a grant.

This crate traverses the verified view without a runtime catalog or historical callback. It owns:

- canonical recorded-history replay results;
- exact transition-trace and safe access-audit derivation;
- deterministic semantic and audit export canonicalization;
- complete source/object closure traversal; and
- callback-free offline verification of portable bundles.

It owns no append path, scheduler, live capability, provider, executor, transport, signer,
filesystem-domain reader, or replay broker. Exact reproduction gives an isolated historical
executable resolver only canonical plan bytes. Candidate comparison instead resolves the
capability-free authoring, certification, and state callbacks sealed by one qualified current
program registry. Recorded verification returns one affine session that privately owns the sole
authoritative verified view. Verification mode renders that session directly; non-verification
modes consume it while strictly binding caller-held semantic export bytes and their expected
`ContentRef` to the same store, tenant, run, semantic head, and closure. Replay never exposes the
view, loads history twice, mints an export grant, or generates a fallback export. Candidate plans
bind all six executable/planner/state/capability references, and replay derives the candidate
identity, plan, and per-transition verdicts itself from the verified history. The recorded stable
entry-point operation selects the current candidate across versioned entry-point ids; callers
cannot supply an operation, entry point, candidate identity, or label. Incompatible candidate
contract metadata produces `NotComparable` without consuming recorded evidence. Exact plans bind
the semantic head and transition prefix, so later authorization or observation audit-tail commits
do not change historical execution input.

Same-store fact completeness is positive only when the authoritative store rechecks the immutable
private scan attestation and its complete dense writer prefix. Portable/offline verification always
returns `UnverifiedPortableBundle`, because included portable bytes cannot prove tenant-wide
omission completeness.

Transition trace paging accepts only the application-decoded optional fixed head, start index, and
limit, then returns the next scalar index; the application alone owns opaque transport cursors.
Cross-run inputs require independent source-run `InspectTrace` authorities on every page; a denied
or authorized-but-missing source emits the same digest-only redacted lineage, while an extra
supplied source authority is rejected.

Access-audit paging likewise fixes one physical journal head. Its public entries are replay-owned
DTOs derived from the store's verified authorization, observation, and ensure-result relationships;
they are not persisted journal entries. The journal retains only `delivery_audit_ref`, while the
public projection annotates a verified returned ensure as pending or terminal without decoding
retained values in the application. Replay accepts only the store seam's decoded head, start index,
and limit and returns the next scalar index; the application alone owns opaque transport cursors.

Portable members use fixed coordinate-derived paths, with the root at `runs.r0000` and dependency
runs sorted by canonical run identity. The manifest contains no self or bundle digest. The sole
transport integrity value is raw SHA-256 over the exact final canonical bundle bytes and is returned
outside those bytes. Export recursively follows the complete source closure fixed by each run's
admission and requires the exact transitive `Export` authority set. A missing source authority is a
denial; after that exact authority is supplied, an absent or corrupt append-only source is export
integrity failure rather than another policy denial.
