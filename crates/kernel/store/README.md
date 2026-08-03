# mfm-store

Sole callback-free fold and atomic persistence port for structured runs.

The fold validates complete raw history, re-certifies the persisted program, enforces writer
lineage and exact heads, checks record logical keys/cursor legality/access linkage/lexical
provenance, validates exact object and fact closure, and derives one `VerifiedStructuredRun`.
That view contains chronology, semantic/journal heads, lexical bindings, fan-out cursors, waiting
or blocked state, and terminal outcome.

`StructuredRunStore::split` yields one non-cloneable writer and cloneable readers. Runtime consumes
the writer. Replay/app use readers. Backends accept only store-validated batches and normalize
positive results against the retained candidate.

Non-empty fact transitions atomically advance a dense store/epoch/tenant publication frontier.
The exact reserved prior-run fact Read captures that frontier, and only its newly committed
authorization receives a one-use purpose-limited scan permit. The scanner verifies exact producer
prefixes, source eligibility, and canonical subject/response/claim material without exposing
generic backend or append authority. Public
verified loads recompute retained positive scanner responses at their recorded barrier.

The crate also defines a separate append-only configured-value history keyed by store, tenant,
entry operation, and target. Deployment owns its writer; application uses a reader. Configuration
is not a sixth run record family.

The memory backend is conformance/test support. Production PostgreSQL lives in
`mfm-storage-postgres` and must implement identical semantics with real SQL.
