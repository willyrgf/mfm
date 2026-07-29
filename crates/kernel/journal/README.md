# mfm-journal

Typed, domain-free persisted values for the recoverability-v2 run journal.

The crate loads the frozen recoverability annex through `mfm-canonical`. Every
value is structurally validated before it can be encoded, decoded, hashed, or
used. It does not copy schema descriptors or schema identifiers and does not
derive persisted wire shapes from Rust serialization.

`mfm-journal` owns record, transition, access-audit, object-binding, fact,
closure, and commit-envelope values. It owns no store IO, fold, scheduler,
callback, replay, or ambient-access authority.

Producer-independent retained metadata uses the single
`mfm_values::RetainedValueContract`, re-exported by this crate. The journal adds producer, exact
byte, artifact, and evidence identity only through its `ValueRef`.
The certified spec owns the complete journal-protocol contract set; this crate
defines the protocol values but has no spec dependency or duplicate protocol
contract aggregate.

Fact emissions persist both a dense actual `emission_ordinal` and the certified
`fact_slot_ordinal` that authorized the emission. A successful settlement keeps
emissions grouped in nondecreasing slot order and never returns to an earlier
slot; fact references and identities continue to use the actual emission
ordinal.

Input lineage is root-only. Nested source selection lives only on
`InputBinding::source_field_path`; a qualified-support root retains its exact
support-graph `member_path` alongside the full producer-bound `ValueRef`.
Admission config, seed, context, and cross-run source manifests are closed,
path-sorted journal values. Their public constructors canonicalize at most
4,096 entries and reject duplicate paths; only the store grants them run
authority by committing the admission. These manifests and the per-transition
input manifest expose zero-argument deterministic retained-contract factories
using their annex schema and the one shared component-object-evidence identity.
The store-authored fact-claim envelope and immutable frozen read intent expose
the same zero-argument protocol factory, with their runtime roles fixed by the
journal contract.
