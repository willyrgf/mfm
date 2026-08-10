# mfm-store

Hostile-input qualification, pure event reduction, compilation, obligation discharge, semantic
open, a private production history adapter, and purpose readers for structured runs.

Qualification validates complete raw history, re-certifies the persisted program via the concrete
`AdmissionVerificationRegistry`, enforces writer lineage and exact heads, checks record logical
keys/cursor legality/access linkage/lexical provenance, validates exact object and fact closure,
and produces immutable typed history. Callback-free `reduce_event` interprets only that history and
derives compact semantic continuation. The compiler authors and compares records and projections;
obligations discharge external/current checks; semantic open audits stored projections; backends
apply sealed plans mechanically. Their completed result is one `VerifiedStructuredRun`.

The private Runtime adapter retains at most one same-reducer verified successor after admission,
mutation, or a non-mutating frontier. Each reuse first compares its journal head with the backend's
indexed current head; a stale or absent entry re-reduces the complete history, so this bounded
optimization cannot become a second authority or hide an external append. The indexed-head read
defines the snapshot point for a non-mutating load; exact-head compare-and-append still rejects a
mutation based on a later external append.

Production assembly consumes one complete `CertifiedProgramRegistry` and a backend, then returns
only `Runtime` plus sealed purpose readers (public-read, trace, audit, replay, export). Each
purpose reader returns only its purpose-sealed evidence newtype (`PublicRunEvidence`,
`TraceRunEvidence`, `AuditRunEvidence`, `RecordedRunEvidence`, `ExportRunEvidence`); none expose
complete `VerifiedStructuredRun` publicly, and cross-purpose evidence substitution is a type error.
Public and recorded-replay evidence expose only a reducer-derived status tag; actionable frontier
details and capability references remain internal to the store.
The explicit offline verifier returns only an opaque recorded-status/export-metadata summary;
the complete `VerifiedStructuredRun` is not part of the crate's public API.
Assembly never returns a store, writer, port, backend, pool, proposal constructor, or append
attempt. Semantic mutation is reachable only through Runtime's `RuntimeHistoryPort`, implemented by
a private adapter.

`RunId` is derived inside the adapter from the `mfm.run-id` semantic domain (store scope, tenant
scope, entry-point operation ID, invocation identity). Callers never supply a trusted run digest.

On certified-root cache miss the adapter calls `AdmissionVerificationRegistry::verify_root` and
consumes private-field `CertifiedProgram` data. There is no public verifier trait or verified-program
constructor.

Non-empty fact transitions atomically advance a dense store/epoch/tenant publication frontier.
The exact reserved prior-run fact Read captures that frontier, and only its newly committed
authorization receives a one-use purpose-limited scan permit.

The crate also defines a separate append-only configured-value history keyed by store, tenant,
entry operation, and target. Deployment owns its writer; application uses a reader. Configuration
is not a sixth run record family.

The memory backend is conformance/test support. Production PostgreSQL lives in
`mfm-storage-postgres` and must implement identical semantics with real SQL.
